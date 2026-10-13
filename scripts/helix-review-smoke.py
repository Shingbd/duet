#!/usr/bin/env python3
"""Real Helix + fake RPC: edit approved review into independent immutable integration draft."""
import fcntl,json,os,pathlib,pty,select,sqlite3,struct,subprocess,sys,tempfile,termios,time
binary=pathlib.Path(sys.argv[1]).resolve();helix=pathlib.Path(sys.argv[2]).resolve()
with tempfile.TemporaryDirectory(prefix='duet-merge-') as directory:
    base=pathlib.Path(directory);project=base/'project';project.mkdir()
    (project/'main.txt').write_text('old baseline\n');(project/'keep.txt').write_text('keep\n')
    for args in [('init','-q'),('add','.'),('-c','user.name=Test','-c','user.email=test@example.com','commit','-qm','base')]:
        subprocess.run(['git','-C',str(project),*args],check=True)
    original=subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD']).decode().strip()
    bin_dir=base/'bin';bin_dir.mkdir();fake=bin_dir/'pi'
    fake.write_text("""#!/usr/bin/env python3
import json,os,pathlib,sys,time
planner=sys.argv[sys.argv.index('--tools')+1]=='read,grep,find,ls'
def emit(x):print(json.dumps(x),flush=True)
for line in sys.stdin:
 r=json.loads(line);reply={'type':'response','id':r['id'],'success':True}
 if r['type']=='get_state':
  reply['data']={'sessionId':'session-'+str(os.getpid()),'model':{'id':'fake'},'isStreaming':False,'isCompacting':False,'pendingMessageCount':0};emit(reply)
 elif r['type']=='prompt':
  reply['data']={'disposition':'started'};emit(reply);emit({'type':'agent_start'})
  if planner:
   node={'id':'change','title':'change source','objective':'update main','scope':['main.txt'],'boundaries':['only own main.txt'],'implementation':'edit main.txt','constraints':['no tests'],'dependencies':[],'acceptance':['inspect diff']}
   text=json.dumps({'reply':'ready','plan':{'schema_version':1,'objective':'goal','constraints':[],'acceptance':['inspect'],'nodes':[node]}})
  else:
   pathlib.Path('main.txt').write_text('first change')
   emit({'type':'tool_execution_end','toolName':'write'});time.sleep(1)
   pathlib.Path('main.txt').write_text('second change\\nnew growth')
   emit({'type':'tool_execution_end','toolName':'edit'});time.sleep(1)
   text='implemented; tests not run'
  emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':text}]}})
  emit({'type':'agent_settled','aborted':False})
 else:emit(reply)
""");fake.chmod(0o700)
    env={**os.environ,'TERM':'xterm-256color','PATH':str(bin_dir)+':'+os.environ['PATH']}
    config=base/'config.json';config.write_text(json.dumps({'helix_path':str(helix)}))
    master,slave=pty.openpty();before=termios.tcgetattr(slave)
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,130,0,0))
    process=None;output=bytearray();database=project/'.duet/workspace.db'
    def launch():return subprocess.Popen([str(binary),'--review-editor','native','--config',str(config),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,env=env)
    def drain(seconds=.1):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if select.select([master],[],[],.01)[0]:output.extend(os.read(master,65536))
    def send(keys):os.write(master,keys);drain(.18)
    def records():
        with sqlite3.connect(database) as db:
            task=json.loads(db.execute('SELECT record FROM tasks').fetchone()[0])
            node=json.loads(db.execute('SELECT record FROM nodes').fetchone()[0])
            ui=db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()
            return task,node,json.loads(ui[0]) if ui else None
    def wait(predicate):
        end=time.monotonic()+12
        while time.monotonic()<end:
            drain(.04)
            try:
                if predicate():return
            except (sqlite3.Error,IndexError,TypeError):pass
            assert process.poll() is None,'TUI exited unexpectedly'
        raise AssertionError('timeout: '+str(records())+' output='+repr(bytes(output[-2500:])))
    def head():return subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD']).decode().strip()
    try:
        process=launch();drain(.4);send(b'1iDiscuss the source change\r')
        wait(lambda:records()[0]['state']=='Ready');send(b's\r')
        wait(lambda:records()[1]['execution']=='Running');send(b'2')
        wait(lambda:len(records()[1]['attempts'][0].get('previews',[]))>=1)
        first=records()[1]['attempts'][0]['previews'][0]
        assert b'old baseline' in output and b'first change' in output and b'NORMAL' in output
        wait(lambda:len(records()[1]['attempts'][0].get('previews',[]))>=2)
        drain(.4)
        assert records()[2]['review_version']!=first,'no v needed'
        wait(lambda:records()[1]['execution']=='Implemented')
        drain(.4);assert b'growth' in output
        assert head()==original and (project/'main.txt').read_text()=='old baseline\n'
        send(b'a\r');wait(lambda:records()[1]['review']=='Approved')
        before_node=records()[1].copy()
        send(b'h');send(b'\r');drain(2)
        assert b'HELIX' in output
        current=project/'.duet/integrations'/records()[0]['id']/'current.json'
        wait(lambda:current.exists())
        draft=json.loads(current.read_text());working=current.parent/draft['nonce']/'worktree/main.txt'
        send(b'%c');send(b'\x1b[200~human Helix repair\x1b[201~');send(b'\x1b');send(b':wq\r')
        wait(lambda:json.loads(current.read_text())['resolved']==['main.txt'])
        assert records()[1]['review']=='Approved' and records()[1]['attempts']==before_node['attempts']
        assert head()==original and (project/'main.txt').read_text()=='old baseline\n'
        assert working.read_text()=='human Helix repair\n',working.read_text()
        task,node,_=records();candidate=project/'.duet/worktrees'/task['id']/'main.txt'
        assert candidate.read_text()=='second change\nnew growth','original candidate changed'
        send(b'\r');assert head()==original
        send(b'\r');assert head()==original
        send(b'\r');wait(lambda:records()[0]['state']=='Merged')
        assert (project/'main.txt').read_text()=='human Helix repair\n'
        send(b'\x1d');send(b'q');assert process.wait(timeout=3)==0;assert termios.tcgetattr(slave)==before
        print('REAL HELIX review passed: approved delivery unchanged, independent draft, immutable manual revision, explicit final merge')
    finally:
        if process and process.poll() is None:process.kill();process.wait()
        os.close(master);os.close(slave)
