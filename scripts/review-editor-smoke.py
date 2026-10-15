#!/usr/bin/env python3
"""Fake RPC + PTY: native review editor, exact unsaved context, readonly chat, explicit patch, fixed revision and merge."""
import fcntl,json,os,pathlib,pty,select,sqlite3,struct,subprocess,sys,tempfile,termios,time
binary=pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='duet-merge-') as directory:
    base=pathlib.Path(directory);project=base/'project';project.mkdir()
    (project/'main.txt').write_text('old baseline\n');(project/'keep.txt').write_text('keep\n')
    for args in [('init','-q'),('add','.'),('-c','user.name=Test','-c','user.email=test@example.com','commit','-qm','base')]:
        subprocess.run(['git','-C',str(project),*args],check=True)
    original=subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD']).decode().strip()
    bin_dir=base/'bin';bin_dir.mkdir();fake=bin_dir/'pi'
    fake.write_text("""#!/usr/bin/env python3
import json,os,pathlib,sys,time
review='--no-tools' in sys.argv\nplanner=not review and sys.argv[sys.argv.index('--tools')+1]=='read,grep,find,ls'
def emit(x):print(json.dumps(x),flush=True)
for line in sys.stdin:
 r=json.loads(line);reply={'type':'response','id':r['id'],'success':True}
 if r['type']=='get_state':
  reply['data']={'sessionId':'session-'+str(os.getpid()),'model':{'id':'fake'},'isStreaming':False,'isCompacting':False,'pendingMessageCount':0};emit(reply)
 elif r['type']=='prompt':
  reply['data']={'disposition':'started'};emit(reply);emit({'type':'agent_start'})
  if review:
   payload=json.loads(r['message'].rsplit('\\n',1)[1]);code=payload['reference']['code']
   assert code=='manual_design','wrong cursor/selection context: '+repr(code)
   assert 'new growth' not in code,'sent extra file text'
   text=json.dumps({'reply':'Discussed unsaved selection','replacement':{'old':code,'new':'better_design'}})
  elif planner:
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
    master,slave=pty.openpty();before=termios.tcgetattr(slave)
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,130,0,0))
    process=None;output=bytearray();database=project/'.duet/workspace.db'
    def launch():return subprocess.Popen([str(binary),'--review-editor','native','--helix',str(base/'missing-hx'),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,env=env)
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
        process=launch();drain(.4);send(b'\x1bOPiDiscuss the source change\r')
        wait(lambda:records()[0]['state']=='Ready');send(b's\r')
        wait(lambda:records()[1]['execution']=='Running');send(b'\x1bOQ')
        wait(lambda:len(records()[1]['attempts'][0].get('previews',[]))>=1)
        first=records()[1]['attempts'][0]['previews'][0]
        assert b'old baseline' in output and b'first change' in output
        wait(lambda:records()[1]['execution']=='Implemented')
        drain(.4);send(b'\t');send(b'i')
        send(b'\x1b[200~manual_design\x1b[201~');send(b'\x1b')
        # Select exact unsaved inserted text from the start, summon without a range dialog.
        send(b'g');send(b'v');send(b'l'*len('manual_design'));send(b'\x07')
        send(b'Discuss this design\r')
        wait(lambda:b'Discussed unsaved selection' in output)
        def workspace():
            with sqlite3.connect(database) as db:
                return json.loads(db.execute("SELECT value FROM ui_state WHERE key='review_workspace'").fetchone()[0])
        wait(lambda:any(b['text'].startswith('manual_design') for b in workspace()[0]))
        assert records()[1]['review']=='Pending'
        assert (project/'main.txt').read_text()=='old baseline\n' and head()==original
        # AI must not apply a suggested patch until Ctrl+p then Enter.
        send(b'\t');send(b'\t');send(b'\x10');send(b'\x1b')
        assert not any(b['text'].startswith('better_design') for b in workspace()[0])
        send(b'\x10');send(b'\r')
        wait(lambda:any(b['text'].startswith('better_design') for b in workspace()[0]))
        # Publishing requires a new immutable attempt; approval isn't inherited.
        old_cp=records()[1]['attempts'][-1]['checkpoint']
        send(b'\x13');wait(lambda:len(records()[1]['attempts'])==2)
        assert records()[1]['review']=='Pending' and records()[1]['attempts'][-1]['checkpoint']!=old_cp
        task,node,_=records();candidate=project/'.duet/worktrees'/task['id']/'main.txt'
        assert candidate.read_text()=='second change\nnew growth','original candidate overwritten'
        send(b'a\r');wait(lambda:records()[1]['review']=='Approved')
        send(b'g');assert head()==original,'preview is not merge'
        send(b'\x1b');assert head()==original
        send(b'g');send(b'\r');assert head()==original,'first Enter only opens risk confirmation'
        send(b'\x1b');assert head()==original
        send(b'g');send(b'\r');send(b'\r')
        wait(lambda:records()[0]['state']=='Merged')
        merged=head();assert merged!=original
        assert records()[0]['merged']['commit']==merged
        assert (project/'main.txt').read_text()=='better_designsecond change\nnew growth'
        assert (project/'keep.txt').read_text()=='keep\n'
        send(b'\x1bOR');drain(.4);assert records()[2]['selected'][0]==merged,'Project view refreshes to merged HEAD'
        send(b'q');assert process.wait(timeout=3)==0;assert termios.tcgetattr(slave)==before
        process=launch();drain(.5)
        assert records()[0]['state']=='Merged' and head()==merged
        send(b'q');assert process.wait(timeout=3)==0
        print('PTY native review passed: full code/deletions, unsaved selection context, no range confirmation, readonly discussion, explicit patch/cancel, fixed human revision, independent approval, untouched candidate, merge/restart')
    finally:
        if process and process.poll() is None:process.kill();process.wait()
        os.close(master);os.close(slave)
