#!/usr/bin/env python3
"""Real TUI + real subprocess/Git verification, temporary fixture, no model calls."""
import os,pty,pathlib,subprocess,tempfile,sqlite3,json,fcntl,termios,struct,time,select,re,sys
binary=pathlib.Path(sys.argv[1]).resolve();hx=pathlib.Path(sys.argv[2]).resolve() if len(sys.argv)>2 else None
with tempfile.TemporaryDirectory(prefix='duet-verify-') as d:
 root=pathlib.Path(d);project=root/'project';project.mkdir()
 def git(*args):return subprocess.check_output(['git','-C',str(project),*args])
 git('init','-q');file=project/'main.txt';file.write_text('base\n');git('add','.');git('-c','user.name=test','-c','user.email=test@example.com','commit','-qm','base');base=git('rev-parse','HEAD').decode().strip()
 file.write_text('delivery\n');git('add','.');git('-c','user.name=test','-c','user.email=test@example.com','commit','-qm','delivery');cp=git('rev-parse','HEAD').decode().strip();head=git('rev-parse','HEAD');index=(project/'.git/index').read_bytes();file.write_text('human dirty\n')
 master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,150,0,0));before=termios.tcgetattr(slave);output=bytearray()
 def launch(initial=False):return subprocess.Popen([str(binary),*(['--helix',str(hx)] if hx and not initial else ['--review-editor','native','--helix',str(root/'missing-hx')]),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,env={**os.environ,'TERM':'xterm-256color','XDG_CONFIG_HOME':str(root/'global')})
 p=launch(True)
 def drain(s):
  end=time.monotonic()+s
  while time.monotonic()<end:
   if select.select([master],[],[],max(0,min(.02,end-time.monotonic())))[0]:output.extend(os.read(master,65536))
 def send(b):os.write(master,b);drain(.08)
 def clean():return re.sub(r'\x1b\[[0-9;?]*[A-Za-z]','',output.decode(errors='replace'))
 db=project/'.duet/workspace.db'
 def node():
  with sqlite3.connect(db) as c:return json.loads(c.execute("select record from nodes where node_id='node'").fetchone()[0])
 def wait(pred):
  end=time.monotonic()+8
  while time.monotonic()<end:
   drain(.03)
   if pred():return
  raise AssertionError(clean()[-3500:])
 def close():
  send(b'\x1d')
  if hx:send(b':editor-stop\r');send(b'\r');drain(.3)
  send(b'q');assert p.wait(timeout=4)==0
 def command(spec,confirm=True):
  send(b':verify '+json.dumps(spec).encode()+b'\r');assert '授权固定版本测试' in clean(),clean()[-3000:];send(b'\r' if confirm else b'\x1b')
 try:
  drain(.4);send(b'q');assert p.wait(timeout=3)==0
  contract={'id':'node','title':'fixed node','objective':'goal','scope':['main.txt'],'boundaries':['keep main'],'implementation':'method','constraints':[],'dependencies':[],'acceptance':['fixed input','passing checks']}
  task={'id':'task','prompt':'verify fixture','sessions':[],'state':'Implemented','baseline':base,'checkpoint':cp,'plan':{'schema_version':1,'objective':'goal','constraints':[],'acceptance':['overall'],'nodes':[contract]}}
  n={'contract':contract,'execution':'Implemented','verification':'Unverified','review':'Pending','reviewed_checkpoint':None,'attempts':[{'base':base,'checkpoint':cp,'changed_files':1,'previews':[],'execution':'Implemented','summary':'not evidence','scope_violations':[],'verification':'Unverified','review':'Pending'}]}
  with sqlite3.connect(db) as c:c.execute('insert into tasks values(?,?)',('task',json.dumps(task)));c.execute('insert into nodes values(?,?,?,?)',('task','node',0,json.dumps(n)))
  p=launch();drain(.5);send(b'\x1d');send(b'1');send(b'j');send(b'2');drain(.3)
  marker=root/'marker';spec={'argv':['python3','-c',f"from pathlib import Path;Path({str(marker)!r}).write_text('ran')"],'criteria':[]}
  command(spec,False);drain(.2);assert not marker.exists() and not node().get('verification_runs')
  send(b'm');assert '标记阅读完成' not in clean(),'removed read action still displayed'
  spec={'argv':['python3','-c',"from pathlib import Path;assert Path('main.txt').read_text()=='delivery\\n';print('FIXED INPUT PASS')"],'criteria':[]}
  command(spec);wait(lambda:node().get('verification_runs',[])[-1]['outcome']=='Passed');assert node()['verification']=='Unverified' and node()['review']=='Pending'
  spec['criteria']=[1,2];command(spec);wait(lambda:node()['verification']=='Passed');assert node()['review']=='Pending'
  assert 'FIXED INPUT PASS' in node()['verification_runs'][-1]['output'],'hidden evidence was deleted'
  assert '验收 / 测试证据' not in clean(),'fixed evidence sidebar still visible'
  command({'argv':['python3','-c','raise SystemExit(2)'],'criteria':[1]});wait(lambda:node()['verification']=='Failed')
  command({'argv':['python3','-c','import time;time.sleep(10)'],'criteria':[1],'timeout_secs':30});send(b'\x18');wait(lambda:node()['verification_runs'][-1]['outcome']=='Cancelled')
  command({'argv':['python3','-c','import time;time.sleep(10)'],'criteria':[1],'timeout_secs':1});wait(lambda:node()['verification_runs'][-1]['outcome']=='TimedOut')
  close();assert termios.tcgetattr(slave)==before
  p=launch();drain(.8);assert node()['verification_runs'][-1]['outcome']=='TimedOut';children=pathlib.Path('/proc/'+str(p.pid)+'/task/'+str(p.pid)+'/children').read_text().split();assert len(children)==(1 if hx else 0),'restored old executing test'
  close();
  assert file.read_text()=='human dirty\n' and git('rev-parse','HEAD')==head and (project/'.git/index').read_bytes()==index
  print('VERIFICATION PTY passed: explicit confirm/cancel, fixed input despite dirty main, unmapped vs covered acceptance, logs/UI, failure/cancel/timeout, restart history no execution, removed read, unchanged main/index/HEAD/approval/terminal')
 finally:
  if p.poll() is None:p.kill();p.wait()
  os.close(master);os.close(slave)
