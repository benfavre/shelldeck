#!/usr/bin/env python3
"""SDTEST-2552 / SDUC-557: opt-in native CEF input, dialogs, transport and profile smoke.

Run after building/staging CEF:
    python3 scripts/test-browser-runtime.py target/debug/browser/0.10.0/shelldeck-browser

Creates only a disposable browser profile and a loopback fixture. Prints its
artifact directory. Never reads or changes the desktop user's browser profile.
Requires a working bundled CEF runtime and the OS Chromium sandbox prerequisites.
"""
import os, sys, subprocess, json, struct, threading, queue, time, tempfile, http.server, socketserver, base64, pathlib
helper = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/shelldeck-browser').resolve()
if not helper.is_file():
    sys.exit(f'Browser helper not found: {helper}')
root = pathlib.Path(tempfile.mkdtemp(prefix='shelldeck-cef-smoke.'))
html = '<!doctype html><meta charset=utf-8><title>ShellDeck native Chromium fixture</title><style>body{font:20px sans-serif;background:#1f1f2d;color:white;padding:20px}input,button,select,a{font:20px sans-serif;margin:8px;padding:8px;display:block}button{background:#bc96ef}#spacer{height:1200px}</style><h1>Live native Chromium</h1><input id=text placeholder="Type here"><button id=counter onclick="this.textContent=++window.clicked">Click</button><select id=select><option>First</option><option>Second</option><option>Third</option></select><button id=dialog onclick="alert(\'Native dialog\')">Dialog</button><input id=upload type=file><a id=download href=/download download=fixture.txt>Download</a><a id=new href=/popup target=_blank rel=opener>New tab</a><div id=spacer></div><p>End of page</p><script>window.clicked=0;window.popupMessage=null;window.addEventListener(\'message\',event=>{if(event.origin===location.origin)window.popupMessage=event.data});document.cookie=\'persist=shelldeck;max-age=3600\';localStorage.setItem(\'persistent\',\'yes\')</script>'

class Handler(http.server.BaseHTTPRequestHandler):

    def do_GET(self):
        if self.path == '/download':
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Disposition', 'attachment; filename="fixture.txt"')
            body = b'ShellDeck CEF download fixture'
        elif self.path == '/popup':
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            body = b'<!doctype html><title>Related browser popup</title><script>window.opener.postMessage("opener-ok",location.origin)</script><p>Related popup</p>'
        elif self.path == '/verify':
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            body = b'<!doctype html><title>Persistent profile verification</title><p>Profile retained</p>'
        else:
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            body = html.encode()
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass
server = socketserver.TCPServer(('127.0.0.1', 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
url = f'http://127.0.0.1:{server.server_address[1]}/'
env = dict(os.environ)
if sys.platform.startswith('linux'):
    env['LD_LIBRARY_PATH'] = str(helper.parent) + os.pathsep + env.get('LD_LIBRARY_PATH', '')
if sys.platform == 'win32':
    env['PATH'] = str(helper.parent) + os.pathsep + env.get('PATH', '')
log = (root / 'stderr.log').open('wb')
proc = subprocess.Popen([str(helper), '--runtime', '--profile', str(root / 'profile')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, env=env)
q = queue.Queue()
events = []
frames = []

def recv(process, packets):
    try:
        while True:
            length = process.stdout.read(4)
            if len(length) != 4:
                raise EOFError(f'helper exited {process.poll()}')
            h = json.loads(process.stdout.read(struct.unpack('<I', length)[0]))
            payload = process.stdout.read(h['payload_len'])
            packets.put((h['message'], payload))
    except Exception as e:
        packets.put(({'error': str(e)}, b''))
threading.Thread(target=recv, args=(proc, q), daemon=True).start()

def send(typ, **kw):
    h = json.dumps({'message': {typ: kw} if kw else typ, 'payload_len': 0}).encode()
    proc.stdin.write(struct.pack('<I', len(h)) + h)
    proc.stdin.flush()

def wait(typ, pred=lambda x: True, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        (message, payload) = q.get(timeout=max(0.01, deadline - time.monotonic()))
        events.append(message)
        (root / 'events.json').write_text(json.dumps(events, indent=2))
        if 'Paint' in message:
            frames.append((message['Paint'], len(payload)))
            (root / 'last-frame.bgra').write_bytes(payload)
        if 'error' in message:
            raise RuntimeError(message)
        if typ in message and pred(message[typ]):
            return (message[typ], payload)
    raise TimeoutError(typ)
request = 0

def cdp(method, params={}, browser_id=1):
    global request
    request += 1
    send('DevTools', browser_id=browser_id, request_id=request, method=method, params=params)
    (r, _) = wait('DevToolsResult', lambda r: r['request_id'] == request)
    if r['error']:
        raise RuntimeError(r)
    return r['result']

def evaluate(js, browser_id=1):
    return cdp('Runtime.evaluate', {'expression': js, 'returnByValue': True, 'userGesture': True},browser_id=browser_id)['result'].get('value')

def eventually(js, predicate, timeout=5, browser_id=1):
    deadline = time.monotonic() + timeout
    while True:
        value = evaluate(js,browser_id=browser_id)
        if predicate(value):
            return value
        if time.monotonic() >= deadline:
            raise AssertionError(f'{js}: {value!r}')
        time.sleep(0.025)

def click(selector):
    box = evaluate(f'(()=>{{let r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();return {{x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)}}}})()')
    for pressed in (True, False):
        send('MouseButton', browser_id=1, **box, button='Left', pressed=pressed, click_count=1, modifiers=0)
try:
    (ready, _) = wait('Ready')
    send('Create', browser_id=1, url=url, width=800, height=700, scale_factor=1.0)
    wait('Created')
    wait('State', lambda state: not state['state']['loading'])
    wait('Paint')
    assert frames[-1][0]['width'] == 800
    click('#text')
    eventually('document.activeElement.id', lambda value: value == 'text')
    send('Text', browser_id=1, text='ShellDeck native input é 🦀')
    eventually('document.querySelector("#text").value', lambda value: value == 'ShellDeck native input é 🦀')
    click('#counter')
    eventually('window.clicked', lambda value: value == 1)
    box = evaluate('(()=>{const r=document.querySelector("#text").getBoundingClientRect();return {x:Math.round(r.x+10),y:Math.round(r.y+10)}})()')
    for pressed in (True, False):
        send('MouseButton',browser_id=1,**box,button='Right',pressed=pressed,click_count=1,modifiers=0)
    menu,_ = wait('ContextMenu')
    assert menu['editable'] is True
    click('#select')
    wait('Popup', lambda p: p['visible'])
    send('Key', browser_id=1, event={'kind': 'RawKeyDown', 'windows_key_code': 40, 'native_key_code': 116, 'modifiers': 0, 'character': None, 'unmodified_character': None, 'is_system_key': False})
    send('Key', browser_id=1, event={'kind': 'RawKeyDown', 'windows_key_code': 13, 'native_key_code': 36, 'modifiers': 0, 'character': None, 'unmodified_character': None, 'is_system_key': False})
    click('#dialog')
    (d, _) = wait('Dialog')
    send('DialogResponse', browser_id=1, dialog_id=d['dialog_id'], accept=True, prompt='')
    click('#upload')
    (chooser, _) = wait('FileChooser')
    upload = root / 'upload.txt'
    upload.write_text('upload fixture')
    send('FileChooserResponse', browser_id=1, chooser_id=chooser['chooser_id'], paths=[str(upload)])
    eventually('document.querySelector("#upload").files[0]?.name', lambda value: value == 'upload.txt')
    click('#download')
    (d, _) = wait('Download')
    download = root / 'download.txt'
    send('DownloadResponse', browser_id=1, download_id=d['download_id'], path=str(download))
    wait('DownloadProgress', lambda d: d['complete'])
    assert download.read_text() == 'ShellDeck CEF download fixture'
    click('#new')
    (child, _) = wait('Created', lambda tab: tab.get('parent_id') == 1)
    assert child['browser_id'] >= 1 << 63
    eventually('window.popupMessage', lambda value: value == 'opener-ok')
    child_id=child['browser_id']
    for pressed in (True,False):
        send('MouseButton',browser_id=child_id,x=25,y=50,button='Left',pressed=pressed,click_count=1,modifiers=0)
    eventually('navigator.userActivation.hasBeenActive',lambda active:active is True,browser_id=child_id)
    evaluate('window.onbeforeunload=event=>{event.preventDefault();event.returnValue="unsaved fixture"};true',browser_id=child_id)
    send('Close',browser_id=child_id)
    dialog,_=wait('Dialog',lambda dialog:dialog['browser_id']==child_id and dialog['kind']=='before_unload')
    send('DialogResponse',browser_id=child_id,dialog_id=dialog['dialog_id'],accept=False,prompt='')
    assert evaluate('Boolean(document.body)',browser_id=child_id)
    send('Close',browser_id=child_id)
    dialog,_=wait('Dialog',lambda dialog:dialog['browser_id']==child_id and dialog['kind']=='before_unload')
    send('DialogResponse',browser_id=child_id,dialog_id=dialog['dialog_id'],accept=True,prompt='')
    wait('Closed',lambda tab:tab['browser_id']==child_id)
    shot = cdp('Page.captureScreenshot', {'format': 'png'})
    (root / 'screenshot.png').write_bytes(base64.b64decode(shot['data']))
    send('Resize', browser_id=1, width=640, height=480, scale_factor=1.5)
    wait('Paint', lambda f: f['width'] == 960 and f['height'] == 720)
    assert evaluate('document.cookie.includes("persist=shelldeck") && localStorage.getItem("persistent")==="yes"')
    send('Shutdown')
    proc.wait(timeout=20)
    assert proc.returncode == 0
    proc = subprocess.Popen([str(helper), '--runtime', '--profile', str(root / 'profile')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, env=env)
    q = queue.Queue()
    threading.Thread(target=recv, args=(proc, q), daemon=True).start()
    wait('Ready')
    send('Create', browser_id=1, url=url + 'verify', width=800, height=700, scale_factor=1.0)
    wait('Created')
    wait('State', lambda state: not state['state']['loading'])
    assert evaluate('document.cookie.includes("persist=shelldeck") && localStorage.getItem("persistent")==="yes"')
    send('Shutdown')
    proc.wait(timeout=20)
    assert proc.returncode == 0
    (root / 'summary.json').write_text(json.dumps({'ready': ready, 'frames': frames, 'events': [list(e)[0] for e in events]}, indent=2))
    print('PASS', root)
except Exception:
    (root / 'events.json').write_text(json.dumps(events, indent=2))
    proc.kill()
    proc.wait()
    log.flush()
    print('FAIL', root)
    print((root / 'stderr.log').read_text(errors='replace')[-8000:])
    raise
finally:
    server.shutdown()
