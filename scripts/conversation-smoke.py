#!/usr/bin/env python3
"""Actual TUI/PTY pause-correct-resume-follow-up test with a fake Pi, no model calls."""
import fcntl
import json
import os
import pathlib
import pty
import select
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time

root = pathlib.Path(__file__).resolve().parents[1]
binary = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / 'target/debug/duet-workbench'
with tempfile.TemporaryDirectory(prefix='duet-chat-') as directory:
    base = pathlib.Path(directory)
    project = base / 'project'
    project.mkdir()
    (project / 'main.rs').write_text('baseline\n')
    for args in [['init', '-q'], ['add', '.'], ['-c', 'user.name=Probe', '-c', 'user.email=probe@localhost', 'commit', '-qm', 'fixture']]:
        subprocess.run(['git', '-C', str(project), *args], check=True, stdout=subprocess.DEVNULL)
    fake = base / 'fake-pi'
    fake.write_text('''#!/usr/bin/env python3
import json,pathlib,sys
active=False
prompts=0
def emit(r): print(json.dumps(r),flush=True)
for line in sys.stdin.buffer:
 r=json.loads(line)
 response={'type':'response','id':r['id'],'success':True}
 if r['type']=='get_state':
  response['data']={'sessionId':'pty-session','model':{'id':'fake'},'isStreaming':active,'isCompacting':False,'pendingMessageCount':0}
  emit(response)
 elif r['type']=='clear_queue':
  response['data']={'steering':[],'followUp':[]}
  emit(response)
 elif r['type']=='abort':
  active=False
  emit({'type':'agent_settled','aborted':True})
  emit(response)
 elif r['type']=='prompt':
  prompts+=1
  active=True
  response['data']={'disposition':'started'}
  emit(response)
  emit({'type':'agent_start'})
  emit({'type':'message_end','message':{'role':'user','content':r['message']}})
  emit({'type':'message_start','message':{'role':'assistant'}})
  emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'toolCall','name':'read'}]}})
  emit({'type':'message_end','message':{'role':'assistant','content':[]}})
  if prompts==1:
   pathlib.Path('main.rs').write_text('draft')
   text='initial plan'
  else:
   assert r['message'] in ['correct the approach', 'explain the change']
   pathlib.Path('main.rs').write_text('corrected')
   text='corrected result' if prompts==2 else 'follow-up answer'
  emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':text}]}})
  if prompts>1:
   active=False
   emit({'type':'agent_settled','aborted':False})
''')
    fake.chmod(0o700)
    master, slave = pty.openpty()
    before = termios.tcgetattr(slave)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 120, 0, 0))
    process = subprocess.Popen([str(binary), 'code', '--repo', str(project), '--prompt', 'initial task',
                                '--allow-write', '--pi', str(fake)], stdin=slave, stdout=slave, stderr=slave,
                               env={**os.environ, 'TERM': 'xterm-256color'})
    output = bytearray()
    database = project / '.duet/workspace.db'

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.01)
            if ready:
                output.extend(os.read(master, 65536))

    def status():
        with sqlite3.connect(database) as db:
            return db.execute('SELECT status FROM runs').fetchone()[0]

    def wait_status(expected):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            drain(0.05)
            if status() == expected:
                return
        raise AssertionError(f'expected {expected}, got {status()}')

    def send(text):
        os.write(master, text)
        drain(0.2)

    try:
        drain(0.5)
        send(b'1p')
        wait_status('Paused')
        worktree = next((project / '.duet/worktrees').iterdir())
        assert (worktree / 'main.rs').read_text() == 'draft'
        send(b'icorrect the approach\r')
        assert status() == 'Paused'
        assert (worktree / 'main.rs').read_text() == 'draft'
        send(b'p')
        wait_status('Settled')
        assert (worktree / 'main.rs').read_text() == 'corrected'
        send(b'iexplain the change\r')
        wait_status('Settled')
        with sqlite3.connect(database) as db:
            messages = db.execute('SELECT role,content FROM messages ORDER BY id').fetchall()
        assert len(messages) == 6, messages
        assert all(content.strip() for _, content in messages)
        assert [role for role, _ in messages] == ['user', 'assistant'] * 3
        assert (project / 'main.rs').read_text() == 'baseline\n'
        assert '你'.encode() in output and b'AI' in output
        assert b'assistant' not in output
        send(b'q')
        assert process.wait(timeout=3) == 0
        assert termios.tcgetattr(slave) == before
        # Reopen: the task chat AND its immutable delivery files must be available.
        process = subprocess.Popen([str(binary), 'open', str(project)],
                                   stdin=slave, stdout=slave, stderr=slave,
                                   env={**os.environ, 'TERM': 'xterm-256color'})
        drain(0.5)
        send(b':changes\r')
        drain(0.4)
        with sqlite3.connect(database) as db:
            saved = json.loads(db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()[0])
        assert saved['selected'][1] == '变更', saved
        send(b']')
        drain(0.4)
        with sqlite3.connect(database) as db:
            saved = json.loads(db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()[0])
            checkpoint = db.execute('SELECT checkpoint FROM runs').fetchone()[0]
        assert saved['selected'] == [checkpoint, '待审 · main.rs'], saved
        send(b'q')
        assert process.wait(timeout=3) == 0
        assert termios.tcgetattr(slave) == before
        print('PTY chat passed: roles, empty filtering, pause, correction, resume, follow-up, delivery restart, terminal restoration')
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
