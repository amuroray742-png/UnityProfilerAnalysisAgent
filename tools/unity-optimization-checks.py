"""Explicit fixed-check regression on the disposable public Unity project only."""
import argparse
import json
from pathlib import Path
import subprocess
import time
import uuid

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--unity', required=True)
args = parser.parse_args()
project = root / '.cache/unity-project-public'
assert (project / '.upaa-public-fixture').is_file()

def command(name, request):
    result = subprocess.run([args.unity, 'command', name, '--request', json.dumps(request),
        '--project-path', str(project), '--format', 'json', '--non-interactive', '--no-pager'],
        capture_output=True, text=True, encoding='utf-8', timeout=20,
        creationflags=subprocess.CREATE_NO_WINDOW)
    envelope = json.loads(result.stdout)
    assert envelope['success'], envelope
    value = envelope['data']['result']
    if isinstance(value, str): value = json.loads(value)
    assert Path(value['projectRoot']).resolve() == project.resolve()
    return value

journal = project / 'Library/UPAA-optimization-check.json'
if journal.exists():
    old = json.loads(journal.read_text(encoding='utf-8-sig'))
    command('upaa_check_cancel', {'checkId': old['id']})

results = []
for test, expected in [('AllocationWorkPreservesVisibleResult', 'passed'),
                       ('AllocationWorkPreservesVisibleResult', 'passed'),
                       ('DeliberateFailureForCheckProtocol', 'failed')]:
    request = {'checkId': str(uuid.uuid4()), 'paths': [], 'tests': ['PublicOptimizationTests.' + test]}
    start = command('upaa_check_start', request)
    assert start['check']['id'] == request['checkId']
    deadline = time.monotonic() + 180
    while True:
        time.sleep(2)
        try: value = command('upaa_check_status', {'checkId': request['checkId']})
        except (AssertionError, subprocess.TimeoutExpired):
            if time.monotonic() >= deadline: raise
            continue
        if value['check']['phase'] == 'finished' and value['journalDurable']: break
        assert time.monotonic() < deadline, value
    assert value['check']['checkStatus'] == expected, value
    assert json.loads(journal.read_text(encoding='utf-8-sig'))['checkStatus'] == expected
    results.append(value)
    print(test + ': ' + expected, flush=True)
(root / '.cache/optimization-editor-checks.json').write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding='utf-8')
