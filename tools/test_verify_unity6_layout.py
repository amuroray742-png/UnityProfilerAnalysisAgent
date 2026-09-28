"""Artificial regression cases for the research verifier; no private capture."""

import copy
import json
from pathlib import Path
import struct
import tempfile
import unittest

from verify_unity6_layout import VerificationError, f32, padded_string, verify, verify_thread


def fixture(gc_bytes=136, raw_id=42, exported_id=None):
    thread = {"thread_index": 0, "thread_id": raw_id if exported_id is None else exported_id,
              "thread_name": "Main Thread", "thread_group_name": "",
              "gc_alloc_total_bytes": gc_bytes,
              "samples": [
                  {"sample_index": 0, "marker_id": 719, "marker_name": "Main Thread",
                   "time_ms": f32(1234567 * f32(1e-6)), "start_time_ms": 3.0,
                   "children_count": 1, "metadata_count": 0},
                  {"sample_index": 1, "marker_id": 903, "marker_name": "GC.Alloc",
                   "time_ms": f32(100 * f32(1e-6)), "start_time_ms": 3.0,
                   "children_count": 0, "metadata_count": 1, "gc_alloc_bytes": gc_bytes}]}
    body = bytearray(struct.pack("<IQ", 1, raw_id))
    body += padded_string("") + padded_string("Main Thread") + struct.pack("<I", 2)
    body += struct.pack("<IfQi", 719, 1234567, 3000000, 1)
    body += struct.pack("<IfQi", 903, 100, 3000000, 0)
    # Empty section, indexed metadata, GC records, empty section, general
    # metadata, index list, two unknown scalars and counted trailing records.
    body += struct.pack("<7I", 0, 0, 1, 1, gc_bytes, 0, 1)
    body += struct.pack("<5I", 1, 1, 3, 4, gc_bytes)
    body += struct.pack("<4I", 0, 1, 0, 0)
    return body, thread


class LayoutTests(unittest.TestCase):
    def test_samples_gc_and_exact_thread_boundary(self):
        body, thread = fixture()
        result = verify_thread(body, thread)
        self.assertEqual(result["gc_bytes"], 136)
        self.assertEqual(result["samples"], 2)
        self.assertEqual(result["header_offset"], 4)
        self.assertEqual(result["next_offset"], len(body))

    def test_real_zero_allocation_is_verified(self):
        body, thread = fixture(0)
        self.assertEqual(verify_thread(body, thread)["gc_bytes"], 0)

    def test_missing_and_ambiguous_headers_fail(self):
        body, thread = fixture()
        for damaged in (body[:20], body + body):
            with self.subTest(size=len(damaged)), self.assertRaisesRegex(VerificationError, "candidates"):
                verify_thread(damaged, thread)

    def test_every_sample_field_is_checked(self):
        body, thread = fixture()
        for field, wrong in (("marker_id", 999), ("time_ms", 8.0),
                             ("start_time_ms", 10.0), ("children_count", 0)):
            modified = copy.deepcopy(thread)
            modified["samples"][0][field] = wrong
            with self.subTest(field=field), self.assertRaisesRegex(VerificationError, field):
                verify_thread(body, modified)

    def test_gc_mismatch_missing_metadata_and_truncated_tail_fail(self):
        body, thread = fixture()
        for field, wrong in (("gc_alloc_bytes", 137), ("metadata_count", 0)):
            modified = copy.deepcopy(thread)
            modified["samples"][1][field] = wrong
            with self.subTest(field=field), self.assertRaises(VerificationError):
                verify_thread(body, modified)
        with self.assertRaises(VerificationError):
            verify_thread(body[:-1], thread)
        # The separate general metadata payload must agree with the GC table.
        damaged = bytearray(body)
        struct.pack_into("<I", damaged, len(damaged) - 20, 137)
        with self.assertRaisesRegex(VerificationError, "GC payload differs"):
            verify_thread(damaged, thread)

    def test_thread_id_conversion_is_explicit_and_exact(self):
        raw = 0x7BE0000030
        exported = 0xFFFFFFFFE0000030
        body, thread = fixture(raw_id=raw, exported_id=exported)
        self.assertEqual(verify_thread(body, thread)["signed_thread_id"], 1)
        thread["thread_id"] = 0x12E0000030
        with self.assertRaises(VerificationError):
            verify_thread(body, thread)

    def test_sentinel_marker_only_allowed_at_root(self):
        body, thread = fixture()
        offset = verify_thread(body, thread)["offset"]
        struct.pack_into("<I", body, offset, 0xFFFFFFFF)
        thread["samples"][0]["marker_id"] = 0
        self.assertEqual(verify_thread(body, thread)["sentinel_roots"], 1)
        struct.pack_into("<I", body, offset + 20, 0xFFFFFFFF)
        thread["samples"][1]["marker_id"] = 0
        with self.assertRaisesRegex(VerificationError, "marker_id"):
            verify_thread(body, thread)

    def test_file_entry_partial_export_and_missing_file(self):
        body, thread = fixture()
        body += struct.pack("<I", 0xAFAFAFAF)
        dump = {"unity_version": "6000.3.23f1", "frame_count": 2000, "frames": [
            {"frame_index": 0, "threads": [thread], "sample_count_total": 2,
             "gc_alloc_bytes_total": 136}]}
        with tempfile.TemporaryDirectory() as directory:
            data_path, dump_path = Path(directory) / "test.data", Path(directory) / "dump.json"
            data_path.write_bytes(struct.pack("<7I", 0x20220328, len(body), 6000, 3, 23, 2, 1) + body)
            dump_path.write_text(json.dumps(dump), encoding="utf-8")
            report = verify(data_path, dump_path)
            self.assertEqual(report["verified_frames"], 1)
            self.assertEqual(report["declared_frames"], 2000)
            self.assertEqual(len(report["data_sha256"]), 64)
            data_path.unlink()
            with self.assertRaises(FileNotFoundError):
                verify(data_path, dump_path)


if __name__ == "__main__":
    unittest.main()
