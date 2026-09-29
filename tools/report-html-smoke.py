"""Read and print a locally exported report in headless Edge; no Agent invocation."""
import argparse
import base64
import json
from pathlib import Path
import socket
import subprocess
import time
import urllib.request


def main():
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument('--html', type=Path, default=root / '.cache/desktop-reports/combined.html')
    args = parser.parse_args()
    output = args.html.resolve().parent
    assert args.html.is_file(), 'Export a report before running this check'
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    def request(method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(f'http://127.0.0.1:{port}' + path, data=data, method=method, headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=60) as response:
            return json.load(response)['value']
    driver = root / '.cache/webdriver/edge/msedgedriver.exe'
    session = None
    with (output / 'html-driver.log').open('w') as log:
        process = subprocess.Popen([str(driver), f'--port={port}', '--host=127.0.0.1'], stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            for _ in range(100):
                try:
                    request('GET', '/status')
                    break
                except OSError:
                    time.sleep(.1)
            session = request('POST', '/session', {'capabilities': {'alwaysMatch': {'browserName': 'MicrosoftEdge', 'ms:edgeOptions': {'args': ['--headless=new', '--disable-gpu', '--no-first-run', '--window-size=1200,900']}}}})['sessionId']
            prefix = '/session/' + session
            request('POST', prefix + '/url', {'url': args.html.resolve().as_uri()})
            state = request('POST', prefix + '/execute/sync', {'script': '''return {text:document.body.innerText,tables:document.querySelectorAll('table').length,code:document.querySelectorAll('pre code').length,unsafe:document.querySelectorAll('script,img,iframe,object').length,overflow:document.documentElement.scrollWidth>innerWidth}''', 'args': []})
            assert '性能诊断报告' in state['text'] and ('C# 源码定位报告' in state['text'] or 'Unity 工程性能定位报告' in state['text'])
            assert state['tables'] and state['code'] and state['unsafe'] == 0 and not state['overflow']
            (output / 'html-reading.png').write_bytes(base64.b64decode(request('GET', prefix + '/screenshot')))
            pdf = base64.b64decode(request('POST', prefix + '/print', {'background': True, 'page': {'width': 21, 'height': 29.7}, 'margin': {'top': 1.5, 'bottom': 1.5, 'left': 1.5, 'right': 1.5}}))
            assert pdf.startswith(b'%PDF-') and len(pdf) > 5000
            (output / 'combined-print.pdf').write_bytes(pdf)
            state.pop('text')
            state.update({'passed': True, 'pdfBytes': len(pdf), 'html': args.html.name})
            (output / 'html-result.json').write_text(json.dumps(state, indent=2), encoding='utf-8')
            print(json.dumps(state))
        finally:
            if session:
                request('DELETE', '/session/' + session)
            process.terminate()
            process.wait(timeout=10)


if __name__ == '__main__':
    main()
