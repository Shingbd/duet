#!/usr/bin/env python3
"""Fake RPC + PTY: native review editor, exact unsaved context, readonly chat, explicit patch, fixed revision and merge."""
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
    cfg=base/'config.json';cfg.write_text(json.dumps({'helix_keepalive':False,'helix_startup':False}))
    def launch():return subprocess.Popen([str(binary),'--config',str(cfg),'--helix',str(helix),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,env=env)
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
    def workspace():
        with sqlite3.connect(database) as db:
            return json.loads(db.execute("SELECT value FROM ui_state WHERE key='review_workspace'").fetchone()[0])
    def snapshot_text():
        cp=records()[1]['attempts'][-1]['checkpoint']
        return subprocess.check_output(['git','-C',str(project),'show',cp+':main.txt']).decode()
    def children():
        return pathlib.Path('/proc/'+str(process.pid)+'/task/'+str(process.pid)+'/children').read_text().strip()
    try:
        process=launch();drain(.4);send(b'1iDiscuss the source change\r')
        wait(lambda:records()[0]['state']=='Ready');send(b's\r')
        wait(lambda:records()[1]['execution']=='Implemented');drain(.5)
        send(b'3');drain(1);assert children(),'real Helix not started'
        editor_pid=children();assert b'HELIX' in output
        send(b'2f');drain(.5) # queued view+fullscreen keys must resolve the target before editing
        wait(lambda:records()[2]['panel']=='Reader')
        send(b'j');send(b'k');send(b'fc') # complete native find; not host fullscreen toggle
        send(b'\x1b[17~');drain(.5);wait(lambda:records()[2]['panel']=='Queue')
        send(b'f');drain(.5);wait(lambda:records()[2]['panel']=='Reader')
        assert children()==editor_pid,'fullscreen restarted editor'
        send(b'\x1d');send(b'\x1b[91;5u');send(b'imanual_design ');send(b'\x1b') # review navigation retains editor focus
        send(b'ggv');send(b'l'*12);send(b'\x07');drain(.6)
        send(b'Discuss this design\r')
        wait(lambda:b'Discussed unsaved selection' in output)
        wait(lambda:any(b['text'].startswith('manual_design') for b in workspace()[0]))
        assert records()[1]['review']=='Pending'
        assert (project/'main.txt').read_text()=='old baseline\n' and head()==original
        # Re-capture normal-mode cursor word from real Helix (not the host's saved cursor).
        send(b'\x1b');send(b'\x1b');send(b'\x07');drain(.5);send(b'Explain cursor word\r')
        wait(lambda:len([m for m in workspace()[1] if m['role']=='assistant'])>=2)
        # Chat escape keeps the actual editor buffer. Explicit preview/cancel must not apply.
        send(b'\x1b');send(b'\x10');drain(.4);send(b'\x1b')
        assert not any(b['text'].startswith('better_design') for b in workspace()[0])
        send(b'\x10');drain(.4);send(b'\r')
        wait(lambda:any(b['text'].startswith('better_design') for b in workspace()[0]))
        assert children()==editor_pid,'patch restarted editor'
        old_cp=records()[1]['attempts'][-1]['checkpoint']
        send(b'\t');send(b'\x13');wait(lambda:len(records()[1]['attempts'])==2)
        assert records()[1]['review']=='Pending' and records()[1]['attempts'][-1]['checkpoint']!=old_cp
        assert snapshot_text().startswith('better_design')
        send(b'\t');start=len(output);send(b'\x04');drain(.5);assert b'old baseline' in output[start:],'deleted baseline comparison missing';send(b'j');send(b'k');send(b'\x04');drain(.3)
        # Second Ctrl+D must return actual editor ownership, not leave a read-only host.
        send(b'iCOMPARE_EXIT_PROBE');send(b'\x1b');send(b'\x19');drain(.5)
        wait(lambda:any('COMPARE_EXIT_PROBE' in b['text'] for b in workspace()[0]))
        send(b'\t');send(b'u');send(b'\x19');drain(.5);wait(lambda:not any('COMPARE_EXIT_PROBE' in b['text'] for b in workspace()[0]))
        send(b'\x1b[17~');drain(.3)
        task,node,_=records();candidate=project/'.duet/worktrees'/task['id']/'main.txt'
        assert candidate.read_text()=='second change\nnew growth'
        # Project/review share the same real process; j/k selects without Enter.
        send(b'3');drain(.5);assert children()==editor_pid
        project_selection=records()[2]['project_tree_selected'];send(b'f');drain(.5)
        wait(lambda:records()[2]['panel']=='Reader');send(b'j');send(b'k');drain(.5)
        assert records()[2]['project_tree_selected']==project_selection,'fullscreen j/k navigated hidden file tree'
        send(b'\x1b[17~');drain(.5);wait(lambda:records()[2]['panel']=='Queue')
        send(b'j');drain(.3);send(b'k');drain(.3);assert children()==editor_pid
        send(b'2');drain(.5);assert children()==editor_pid
        # Close all saved buffers, then independently approve and merge fixed revision.
        send(b'\t');send(b':wqa\r');drain(.8)
        send(b'a\r');wait(lambda:records()[1]['review']=='Approved')
        send(b'g');send(b'\r');assert head()==original
        send(b'\r');wait(lambda:records()[0]['state']=='Merged')
        assert (project/'main.txt').read_text().startswith('better_design')
        assert candidate.read_text()=='second change\nnew growth'
        send(b'q');assert process.wait(timeout=4)==0;assert termios.tcgetattr(slave)==before
        process=launch();drain(.5);assert not children(),'restart automatically launched editor'
        send(b'q');assert process.wait(timeout=4)==0
        print('REAL HELIX unified passed: review/project same PID, unsaved selection bridge, readonly AI, patch cancel/apply, new fixed revision, unchanged candidate/main before merge, independent approval, restart safety')
    except Exception:
        print(bytes(output[-10000:]).decode(errors='replace'))
        raise
    finally:
        if process and process.poll() is None:process.kill();process.wait()
        os.close(master);os.close(slave)
