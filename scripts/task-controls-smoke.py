#!/usr/bin/env python3
"""Fake RPC + real PTY: discussion, restart/start, live diff/pin, node/task discard."""
import fcntl,json,os,pathlib,pty,select,sqlite3,struct,subprocess,sys,tempfile,termios,time

binary=pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='duet-controls-') as directory:
    base=pathlib.Path(directory)
    project=base/'project';project.mkdir()
    (project/'base.txt').write_text('baseline\n')
    for args in [('init','-q'),('add','.'),('-c','user.name=Test','-c','user.email=test@example.com','commit','-qm','base')]:
        subprocess.run(['git','-C',str(project),*args],check=True)
    bin_dir=base/'bin';bin_dir.mkdir()
    fake=bin_dir/'pi'
    fake.write_text("""#!/usr/bin/env python3
import json,os,pathlib,sys,time
planner=sys.argv[sys.argv.index('--tools')+1]=='read,grep,find,ls'
def emit(x): print(json.dumps(x),flush=True)
for line in sys.stdin:
 r=json.loads(line);reply={'type':'response','id':r['id'],'success':True}
 if r['type']=='get_state':
  reply['data']={'sessionId':'session-'+str(os.getpid()),'model':{'id':'fake'},'isStreaming':False,'isCompacting':False,'pendingMessageCount':0};emit(reply)
 elif r['type']=='prompt':
  reply['data']={'disposition':'started'};emit(reply);emit({'type':'agent_start'})
  if planner:
   nodes=[{'id':'n'+str(i),'title':'node '+str(i),'objective':'write file','scope':['file'+str(i)+'.txt'],'boundaries':['only own this file'],'implementation':'write source','constraints':['no tests'],'dependencies':[] if i==1 else ['n1'],'acceptance':['inspect diff']} for i in (1,2)]
   text=json.dumps({'reply':'Architecture and boundaries are ready.','plan':{'schema_version':1,'objective':'goal','constraints':[],'acceptance':['inspect files'],'nodes':nodes}})
  else:
   node=json.loads(r['message'].split('当前节点契约：')[1].split('明确补充意见：')[0])
   path=pathlib.Path(node['scope'][0]);path.write_text(node['id']+' first')
   emit({'type':'tool_execution_end','toolName':'write'});time.sleep(1.4)
   path.write_text(node['id']+' second')
   emit({'type':'tool_execution_end','toolName':'edit'});time.sleep(1.4)
   text='implemented '+node['id']
  emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':text}]}})
  emit({'type':'agent_settled','aborted':False})
 else:emit(reply)
""")
    fake.chmod(0o700)
    env={**os.environ,'TERM':'xterm-256color','PATH':str(bin_dir)+':'+os.environ['PATH']}
    master,slave=pty.openpty();before=termios.tcgetattr(slave)
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',32,120,0,0))
    process=None;output=bytearray();database=project/'.duet/workspace.db'
    def launch():
        return subprocess.Popen([str(binary),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,env=env)
    def drain(seconds=.1):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if select.select([master],[],[],.01)[0]:output.extend(os.read(master,65536))
    def send(keys):
        os.write(master,keys);drain(.15)
    def records():
        with sqlite3.connect(database) as db:
            tasks=[json.loads(r[0]) for r in db.execute('SELECT record FROM tasks')]
            nodes=[json.loads(r[0]) for r in db.execute('SELECT record FROM nodes ORDER BY position')]
            ui=db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()
            return tasks,nodes,json.loads(ui[0]) if ui else None
    def wait(predicate):
        end=time.monotonic()+12
        while time.monotonic()<end:
            drain(.04)
            try:
                if predicate():return
            except (sqlite3.Error,IndexError,KeyError):pass
            assert process.poll() is None,'TUI unexpectedly exited'
        raise AssertionError('state timed out: '+str(records()))
    try:
        process=launch();drain(.4);send(b'1iDiscuss architecture and create two nodes\r')
        wait(lambda:records()[0][0]['state']=='Ready')
        candidate=next((project/'.duet/worktrees').iterdir())
        assert not (candidate/'file1.txt').exists()
        send(b's');assert not (candidate/'file1.txt').exists(),'s requires confirmation'
        send(b'\x1b');send(b'q');assert process.wait(timeout=3)==0
        assert termios.tcgetattr(slave)==before
        process=launch();drain(.4)
        assert records()[0][0]['state']=='Ready','restart must not execute'
        send(b'1s\r')
        wait(lambda:records()[1][0]['execution']=='Running')
        send(b'2')
        wait(lambda:len(records()[1][0]['attempts'][0].get('previews',[]))>=1)
        drain(.15)
        assert b'+n1 first' in output,'live reader must show first immutable diff'
        first=records()[1][0]['attempts'][0]['previews'][0]
        send(b'v')
        wait(lambda:len(records()[1][0]['attempts'][0].get('previews',[]))>=2)
        drain(.4)
        assert records()[2]['review_version']==first,'pinned reader must not advance'
        wait(lambda:all(n['execution']=='Implemented' for n in records()[1]))
        send(b'v');drain(.4)
        assert b'second' in output
        assert records()[2]['selected'][0]==records()[1][0]['attempts'][-1]['checkpoint']
        send(b'1\t\tj')
        wait(lambda:records()[2]['task_selection']['node_id']=='n2')
        send(b'd');assert len(records()[1])==2,'deletion requires confirmation'
        send(b'\x1b');send(b'd\r')
        wait(lambda:len(records()[1])==1)
        assert (candidate/'file1.txt').read_text()=='n1 second'
        assert not (candidate/'file2.txt').exists()
        send(b'd\r')
        wait(lambda:not records()[0])
        assert not candidate.exists()
        assert (project/'base.txt').read_text()=='baseline\n'
        send(b'q');assert process.wait(timeout=3)==0
        assert termios.tcgetattr(slave)==before
        print('PTY task controls passed: read-only discussion, saved-plan start, live preview, pin, node rollback, task deletion, source safety')
    finally:
        if process and process.poll() is None:process.kill();process.wait()
        os.close(master);os.close(slave)
