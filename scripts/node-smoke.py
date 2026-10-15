#!/usr/bin/env python3
"""Real PTY, deterministic fake RPC: node planning, approval, backpressure and restart."""
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
binary = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv)>1 else root/'target/debug/duet-workbench'
with tempfile.TemporaryDirectory(prefix='duet-nodes-') as directory:
    base=pathlib.Path(directory)
    project=base/'project'
    project.mkdir()
    (project/'base.txt').write_text('baseline\n')
    for args in [['init','-q'],['add','.'],['-c','user.name=Probe','-c','user.email=probe@localhost','commit','-qm','fixture']]:
        subprocess.run(['git','-C',str(project),*args],check=True,stdout=subprocess.DEVNULL)
    fake=base/'fake-pi'
    fake.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
planner=sys.argv[sys.argv.index('--tools')+1]=='read,grep,find,ls'
def emit(r): print(json.dumps(r),flush=True)
for line in sys.stdin:
 r=json.loads(line); response={'type':'response','id':r['id'],'success':True}
 if r['type']=='get_state':
  response['data']={'sessionId':'nodes-'+str(os.getpid()),'model':{'id':'fake'},'isStreaming':False,'isCompacting':False,'pendingMessageCount':0};emit(response)
 elif r['type']=='prompt':
  response['data']={'disposition':'started'};emit(response);emit({'type':'agent_start'})
  if planner:
   nodes=[{'id':'n'+str(i),'title':'node '+str(i),'objective':'implement node '+str(i),'scope':['file'+str(i)+'.txt'],'boundaries':['only implement this node; preserve other behavior'],'implementation':'write the requested file','constraints':['no tests or merge'],'dependencies':[] if i==1 else ['n'+str(i-1)],'acceptance':['review content']} for i in range(1,4)]
   text=json.dumps({'schema_version':1,'objective':'whole requirement','constraints':['keep baseline'],'acceptance':['all nodes reviewed'],'nodes':nodes})
  else:
   node=json.loads(r['message'].split('\\n当前节点契约：')[1].split('\\n明确补充意见：')[0])
   pathlib.Path(node['scope'][0]).write_text(node['id']+'\\n');text='implemented '+node['id']+'; tests not run'
  emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':text}]}});emit({'type':'agent_settled','aborted':False})
 else: emit(response)
''')
    fake.chmod(0o700)
    master,slave=pty.openpty()
    before=termios.tcgetattr(slave)
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',32,120,0,0))
    env={**os.environ,'TERM':'xterm-256color'}
    process=subprocess.Popen([str(binary),'--review-editor','native','code','--repo',str(project),'--prompt','whole requirement','--allow-write','--pi',str(fake)],stdin=slave,stdout=slave,stderr=slave,env=env)
    database=project/'.duet/workspace.db'
    def drain(seconds=0.12):
        until=time.monotonic()+seconds
        while time.monotonic()<until:
            if select.select([master],[],[],0.02)[0]: os.read(master,65536)
    def send(keys):
        os.write(master,keys)
        drain()
    def records():
        with sqlite3.connect(database) as db:
            nodes=[json.loads(row[0]) for row in db.execute('SELECT record FROM nodes ORDER BY position')]
            status=db.execute('SELECT status FROM runs').fetchone()[0]
        return nodes,status
    def wait(predicate):
        until=time.monotonic()+8
        while time.monotonic()<until:
            drain()
            try:
                if predicate(): return
            except (sqlite3.Error,IndexError): pass
            assert process.poll() is None,'TUI exited before predicate'
        raise AssertionError('timed out waiting for node state')
    try:
        wait(lambda: records()[1]=='Paused' and len(records()[0])==3)
        nodes,_=records()
        assert nodes[0]['execution']=='Implemented' and nodes[1]['execution']=='Implemented'
        assert nodes[2]['execution']=='Planned'
        assert all(n['verification']=='Unverified' for n in nodes)
        send(b'2')
        send(b':approve-node\r')
        assert records()[0][0]['review']=='Pending','command must require confirmation'
        send(b'\x1b')
        assert records()[0][0]['review']=='Pending'
        send(b'a')
        send(b'\r')
        wait(lambda: records()[0][0]['review']=='Approved')
        assert records()[0][2]['execution']=='Planned','approval must not implicitly resume'
        send(b'p')
        wait(lambda: records()[1]=='Settled' and records()[0][2]['execution']=='Implemented')
        assert not (project/'file1.txt').exists(),'never auto-merge'
        drain(0.5)
        with sqlite3.connect(database) as db:
            saved=json.loads(db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()[0])
        nodes,_=records()
        first=nodes[0]['attempts'][0]['checkpoint']
        assert saved['review_version']==first
        assert saved['selected']==[first,'file1.txt'],'incoming node must not steal review version'
        send(b'q')
        assert process.wait(timeout=3)==0
        assert termios.tcgetattr(slave)==before
        process=subprocess.Popen([str(binary),'--review-editor','native','open',str(project)],stdin=slave,stdout=slave,stderr=slave,env=env)
        drain(0.6)
        send(b'}')
        drain(0.5)
        with sqlite3.connect(database) as db:
            saved=json.loads(db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()[0])
        assert saved['task_selection']['node_id']=='n2'
        assert saved['selected'][1]=='file2.txt'
        send(b'q')
        assert process.wait(timeout=3)==0
        assert termios.tcgetattr(slave)==before
        if len(sys.argv)>2:
            cfg=base/'helix-nav.json';cfg.write_text(json.dumps({'helix_startup':False,'helix_keepalive':False}))
            process=subprocess.Popen([str(binary),'--config',str(cfg),'--helix',str(pathlib.Path(sys.argv[2]).resolve()),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,env=env)
            drain(.6);send(b'f');drain(1)
            def selected_node():
                with sqlite3.connect(database) as db:return json.loads(db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()[0])['task_selection']['node_id']
            def editor_children():return pathlib.Path(f'/proc/{process.pid}/task/{process.pid}/children').read_text().strip()
            editor_pid=editor_children();assert editor_pid
            send(b'iNODE_TWO_DRAFT');send(b'\x1b')
            send(b'\x1b[93;6u');wait(lambda:selected_node()=='n3');drain(.4)
            send(b'iNODE_THREE_DRAFT');send(b'\x1b')
            send(b'\x1b[123;5u');wait(lambda:selected_node()=='n2');drain(.3)
            send(b'iRETURNED_TWO');send(b'\x1b')
            for key,node in [(b'\x1b[6;6~','n3'),(b'\x1b[6;6~','n1'),(b'\x1b[5;6~','n3'),(b'\x1b[91;6u','n2')]:
                send(key);wait(lambda:selected_node()==node);drain(.3)
            assert editor_children()==editor_pid,'node switch restarted Helix'
            send(b':wqa\r');drain(1);wait(lambda:not editor_children())
            manifests=list((project/'.duet/helix-workspaces').glob('*/manifest.json'));assert len(manifests)==1
            bindings=json.loads(manifests[0].read_text())['bindings']
            bodies={b['identity']['node']:pathlib.Path(b['file']).read_text() for b in bindings if b['identity'].get('node')}
            assert 'NODE_TWO_DRAFT' in bodies['n2'] and 'RETURNED_TWO' in bodies['n2'],bodies
            assert 'NODE_THREE_DRAFT' in bodies['n3'],bodies
            assert not any((project/f'file{i}.txt').exists() for i in range(1,4)),'main files changed'
            assert [n['review'] for n in records()[0]]==['Approved','Pending','Pending']
            send(b'q');assert process.wait(timeout=3)==0;assert termios.tcgetattr(slave)==before
        print('PTY nodes passed: readonly plan, staged delivery, explicit approval, real backpressure, pinned review, restart, node switch')
    finally:
        if process.poll() is None: process.kill();process.wait()
        os.close(master);os.close(slave)
