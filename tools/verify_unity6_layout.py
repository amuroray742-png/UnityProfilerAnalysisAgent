"""Dump-assisted Unity 6000.3 layout research; NOT a production decoder.

Every exported thread must have exactly one matching header and all samples and
GC records must agree. Offsets and marker IDs come from the supplied dump, not
from a particular recording. Unknown layouts fail closed. No capture content is
written to the report; only counts, offsets, and representation differences.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import time


class VerificationError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise VerificationError(message)


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


def word(body, offset):
    require(0 <= offset <= len(body) - 4, f"word outside body: {offset}")
    return struct.unpack_from("<I", body, offset)[0]


def fingerprint(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def padded_string(value):
    raw = value.encode("utf-8") + b"\0"
    return raw + bytes((-len(raw)) % 4)


def locate_thread(body, thread):
    """Reference-assisted location, including the Editor's signed-32-bit ID case.

    A low-32 match is accepted only if the dump ID is exactly the sign extension
    of the raw ID's low 32 bits. Multiple matches always fail, never pick first.
    """
    name = padded_string(thread["thread_name"])
    group = padded_string(thread["thread_group_name"])
    candidates = []
    search = 0
    while True:
        pos = body.find(name, search)
        if pos < 0:
            break
        search = pos + 1
        start = pos - len(group) - 8
        count_pos = pos + len(name)
        if start < 0 or start % 4 or count_pos + 4 > len(body):
            continue
        if body[start + 8:pos] != group:
            continue
        raw_id = struct.unpack_from("<Q", body, start)[0]
        signed_low = struct.unpack("<i", struct.pack("<I", raw_id & 0xFFFFFFFF))[0]
        exported_id = thread["thread_id"]
        if exported_id not in (raw_id, signed_low % (1 << 64)):
            continue
        if word(body, count_pos) == len(thread["samples"]):
            candidates.append((count_pos + 4, raw_id != exported_id))
    require(len(candidates) == 1,
            f"thread header candidates={len(candidates)}, expected exactly one")
    return candidates[0]


def verify_thread(body, thread):
    offset, converted_id = locate_thread(body, thread)
    samples = thread["samples"]
    end = offset + len(samples) * 20
    require(end <= len(body), "sample table exceeds body")
    sentinel_roots = 0
    stack = []
    gc = {}
    for index, expected in enumerate(samples):
        require(expected["sample_index"] == index, f"sample[{index}] index")
        mid, ns, start, children = struct.unpack_from("<IfQi", body, offset + index * 20)
        while stack and stack[-1] == 0:
            stack.pop()
        is_root = not stack
        if stack:
            stack[-1] -= 1
        require(children >= 0 and children <= len(samples) - index - 1,
                f"sample[{index}] child boundary")
        stack.append(children)
        sentinel = mid == 0xFFFFFFFF and expected["marker_id"] == 0 and is_root
        require(mid == expected["marker_id"] % (1 << 32) or sentinel,
                f"sample[{index}] marker_id")
        sentinel_roots += int(sentinel)
        require(math.isfinite(ns) and ns >= 0, f"sample[{index}] invalid time")
        # Editor exposes float milliseconds; conversion uses a float32 scale.
        require(f32(ns * f32(1e-6)) == expected["time_ms"], f"sample[{index}] time_ms")
        require(f32(start / 1e6) == expected["start_time_ms"],
                f"sample[{index}] start_time_ms")
        require(children == expected["children_count"], f"sample[{index}] children_count")
        if expected["marker_name"] == "GC.Alloc":
            require(expected["metadata_count"] > 0 and "gc_alloc_bytes" in expected,
                    f"sample[{index}] GC metadata missing in reference")
            gc[index] = expected["gc_alloc_bytes"]
    require(not any(stack), "sample tree not closed")

    # Observed prefix after samples: empty section, indexed 8-byte records,
    # then counted (sample_index, allocation_bytes) GC records. Do not scan.
    require(word(body, end) == 0, "unsupported nonempty post-sample section")
    pair_count = word(body, end + 4)
    gc_count_pos = end + 8 + pair_count * 8
    require(gc_count_pos + 4 <= len(body), "indexed metadata exceeds body")
    indexed_metadata = set()
    for entry in range(pair_count):
        sample_index = word(body, end + 8 + entry * 8)
        require(sample_index < len(samples) and sample_index not in indexed_metadata,
                f"indexed metadata[{entry}] sample_index")
        indexed_metadata.add(sample_index)
    count = word(body, gc_count_pos)
    require(count == len(gc), f"GC metadata count: binary={count}, reference={len(gc)}")
    require(gc_count_pos + 4 + count * 8 <= len(body), "GC metadata exceeds body")
    actual = {}
    for entry in range(count):
        index, size = struct.unpack_from("<II", body, gc_count_pos + 4 + entry * 8)
        require(index not in actual, f"duplicate GC metadata index {index}")
        actual[index] = size
    require(actual == gc, "GC sample indices or byte values differ")
    require(sum(actual.values()) == thread["gc_alloc_total_bytes"], "thread GC total")
    cursor = gc_count_pos + 4 + count * 8
    require(word(body, cursor) == 0, "unsupported nonempty post-GC section")
    cursor += 4
    metadata_count = word(body, cursor)
    cursor += 4
    seen = set()
    for entry in range(metadata_count):
        index, field_count = word(body, cursor), word(body, cursor + 4)
        cursor += 8
        require(index < len(samples), f"metadata[{entry}] sample_index")
        if field_count:
            require(index not in seen, f"metadata[{entry}] duplicate sample_index")
            seen.add(index)
            require(field_count == samples[index]["metadata_count"], f"metadata[{entry}] field count")
        else:
            require(index == 0, f"metadata[{entry}] unknown empty record")
        for field in range(field_count):
            # Type is retained as an unknown tag; only the length is interpreted.
            tag = word(body, cursor)
            size = word(body, cursor + 4)
            require(cursor + 8 + size <= len(body), f"metadata[{entry}][{field}] payload exceeds body")
            if index in gc:
                require(field_count == 1 and tag == 3 and size == 4,
                        f"metadata[{entry}] unsupported GC payload")
                require(word(body, cursor + 8) == gc[index],
                        f"metadata[{entry}] GC payload differs from indexed GC bytes")
            cursor += 8 + ((size + 3) & ~3)
            require(cursor <= len(body), f"metadata[{entry}][{field}] payload exceeds body")
    expected_metadata = {i for i, sample in enumerate(samples) if sample["metadata_count"] > 0}
    require(seen == expected_metadata, "general metadata sample coverage differs")
    index_count = word(body, cursor)
    cursor += 4
    require(cursor + index_count * 4 <= len(body), "index list exceeds body")
    for entry in range(index_count):
        require(word(body, cursor + entry * 4) < len(samples), "index list sample_index")
    cursor += index_count * 4
    # Meaning of these two scalar fields and 12-byte records is still unknown.
    word(body, cursor)
    word(body, cursor + 4)
    record_count = word(body, cursor + 8)
    cursor += 12 + record_count * 12
    require(cursor <= len(body), "thread trailing records exceed body")
    header_offset = offset - 4 - len(padded_string(thread["thread_name"])) - len(padded_string(thread["thread_group_name"])) - 8
    return {"offset": offset, "header_offset": header_offset, "next_offset": cursor,
            "samples": len(samples), "gc_bytes": sum(actual.values()),
            "signed_thread_id": int(converted_id), "sentinel_roots": sentinel_roots}


def verify(data_path, dump_path):
    started = time.perf_counter()
    with open(dump_path, encoding="utf-8-sig") as stream:
        dump = json.load(stream)
    require(dump["frames"], "reference has no exported frames")
    targets = {frame["frame_index"]: frame for frame in dump["frames"]}
    require(len(targets) == len(dump["frames"]), "duplicate reference frame index")
    results = []
    block_index = 0
    with open(data_path, "rb") as stream:
        while targets:
            header = stream.read(28)
            require(len(header) == 28, f"missing block {block_index}")
            require(word(header, 0) == 0x20220328, f"block {block_index}: unsupported magic")
            require(word(header, 8) == 6000 and word(header, 12) == 3,
                    f"block {block_index}: only Unity 6000.3 is in research scope")
            release = {0: "a", 1: "b", 2: "f", 3: "c"}.get(word(header, 20), "?")
            version = f"6000.3.{word(header, 16)}{release}{word(header, 24)}"
            require(version == dump["unity_version"], "binary/reference Unity versions differ")
            size = word(header, 4)
            require(size <= Path(data_path).stat().st_size - stream.tell(), "truncated body")
            if block_index not in targets:
                stream.seek(size, 1)
                block_index += 1
                continue
            body = stream.read(size)
            require(size >= 4 and word(body, size - 4) == 0xAFAFAFAF, "frame end marker")
            frame = targets.pop(block_index)
            per_thread = []
            for thread in frame["threads"]:
                try:
                    per_thread.append(verify_thread(body, thread))
                except (VerificationError, KeyError) as error:
                    raise VerificationError(
                        f"frame[{block_index}] thread[{thread['thread_index']}]: {error}") from error
            require(len({entry["offset"] for entry in per_thread}) == len(per_thread),
                    f"frame[{block_index}] threads share sample tables")
            require(per_thread, f"frame[{block_index}] no threads")
            require(word(body, per_thread[0]["header_offset"] - 4) == len(per_thread),
                    f"frame[{block_index}] thread section count")
            for previous, following in zip(per_thread, per_thread[1:]):
                require(previous["next_offset"] == following["header_offset"],
                        f"frame[{block_index}] thread boundary discontinuity")
            require(sum(entry["gc_bytes"] for entry in per_thread) == frame["gc_alloc_bytes_total"],
                    f"frame[{block_index}] GC total")
            require(sum(entry["samples"] for entry in per_thread) == frame["sample_count_total"],
                    f"frame[{block_index}] sample total")
            results.append({"frame_index": block_index, "threads": per_thread})
            block_index += 1
    flattened = [thread for frame in results for thread in frame["threads"]]
    return {"scope": "dump-assisted-layout-only", "unity_version": dump["unity_version"],
            "data_sha256": fingerprint(data_path), "dump_sha256": fingerprint(dump_path),
            "data_bytes": Path(data_path).stat().st_size, "dump_bytes": Path(dump_path).stat().st_size,
            "declared_frames": dump["frame_count"], "verified_frames": len(results),
            "verified_threads": len(flattened), "verified_samples": sum(t["samples"] for t in flattened),
            "gc_bytes": sum(t["gc_bytes"] for t in flattened),
            "signed_thread_ids": sum(t["signed_thread_id"] for t in flattened),
            "sentinel_roots": sum(t["sentinel_roots"] for t in flattened),
            "elapsed_seconds": round(time.perf_counter() - started, 3), "frames": results}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", required=True, type=Path)
    parser.add_argument("--dump", required=True, type=Path)
    parser.add_argument("--report", type=Path, help="Optional summary report, no capture sample content")
    args = parser.parse_args()
    report = verify(args.data, args.dump)
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in report.items() if k != "frames"}, indent=2))


if __name__ == "__main__":
    main()
