"""Generate manual acceptance inputs from the public synthetic fixture only."""
import copy
import json
from pathlib import Path

root = Path(__file__).resolve().parents[1]
base = json.loads((root / 'src-tauri/tests/fixtures/editor-dump.json').read_text(encoding='utf-8'))
output = root / '.cache/manual-fixtures'
output.mkdir(parents=True, exist_ok=True)
cases = {'normal': base}
partial = copy.deepcopy(base)
partial['frames'][0]['threads'][0]['samples'][2]['metadata_count'] = 0
cases['partial-gc'] = partial
invalid = copy.deepcopy(base)
invalid['frames'][0]['threads'][0]['samples'][2]['sample_index'] = 999
cases['invalid-tree'] = invalid
zero = copy.deepcopy(base)
zero['frames'] = [zero['frames'][1]]
cases['zero-gc'] = zero
paged = copy.deepcopy(base)
frame = paged['frames'][0]
frame['threads'] = frame['threads'][:1]
thread = frame['threads'][0]
root_sample = copy.deepcopy(thread['samples'][0])
root_sample['children_count'] = 210
samples = [root_sample]
for index in range(1, 211):
    sample = copy.deepcopy(root_sample)
    sample.update(sample_index=index, marker_id=113, marker_name='SyntheticWork',
                  time_ms=0.01, start_time_ms=index * 0.01, children_count=0)
    samples.append(sample)
thread.update(samples=samples, gc_alloc_total_bytes=0)
frame.update(sample_count_total=211, gc_alloc_bytes_total=0)
paged['frames'] = [frame]
cases['pagination'] = paged
for name, data in cases.items():
    path = output / f'{name}.json'
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding='utf-8')
    print(path.relative_to(root))
