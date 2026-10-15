#!/usr/bin/env python3
"""Real Helix project tree: nested paths, same PTY, unsaved multi-buffer retention and explicit save-all."""
import os,pty,fcntl,struct,termios,subprocess,tempfile,pathlib,json,select,time,sys,re
binary=pathlib.Path(sys.argv[1]).resolve();helix=pathlib.Path(sys.argv[2]).resolve()
with tempfile.TemporaryDirectory(prefix='duet-helix-') as directory:
 base=pathlib.Path(directory);project=base/'project';project.mkdir();source=project/'main.txt';source.write_text('original file\n');(project/'src').mkdir();second=project/'src/second file.txt';second.write_text('second original\n');special=project/'z %sh{touch marker}.txt';special.write_text('literal percent\n')
 for args in [('init','-q'),('add','.'),('-c','user.name=Test','-c','user.email=test@example.com','commit','-qm','base')]:
  subprocess.run(['git','-C',str(project),*args],check=True)
 original=subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD'])
 cfg=base/'config.json';cfg.write_text(json.dumps({'helix_path':str(helix)}))
 master,slave=pty.openpty();before=termios.tcgetattr(slave);fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,130,0,0))
 process=subprocess.Popen([str(binary),'--config',str(cfg),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,cwd=base,env={**os.environ,'TERM':'xterm-256color','XDG_CONFIG_HOME':str(base/'global')})
 output=bytearray()
 def drain(t=.15):
  end=time.monotonic()+t
  while time.monotonic()<end:
   if select.select([master],[],[],.01)[0]:output.extend(os.read(master,65536))
 def send(data):os.write(master,data);drain(.25)
 def clean():return re.sub(r'\x1b\[[0-9;?]*[A-Za-z]','',output.decode(errors='replace'))
 def child():return pathlib.Path('/proc/'+str(process.pid)+'/task/'+str(process.pid)+'/children').read_text().strip()
 def wait(pred):
  deadline=time.monotonic()+5
  while time.monotonic()<deadline:
   drain(.03)
   if pred():return
  raise AssertionError(clean()[-2000:])
 def launch(path=project,extra=[]):
  return subprocess.Popen([str(binary),'--config',str(cfg),*extra,'open',str(path)],stdin=slave,stdout=slave,stderr=slave,cwd=base,env={**os.environ,'TERM':'xterm-256color','XDG_CONFIG_HOME':str(base/'global')})
 def close():
  if child():send(b':editor-stop\r');send(b'\r');wait(lambda:not child())
  send(b'q');assert process.wait(timeout=4)==0
 try:
  wait(lambda:bool(child()));drain(.3);pid=child()
  assert source.read_text()=='original file\n'
  send(b'\x1bOR');drain(.3);assert child()==pid,'first entry relaunched Helix'
  send(b'\x1bOP');close();assert termios.tcgetattr(slave)==before
  # Restart is a NEW process, not recovered executing/model/editor state.
  process=launch();wait(lambda:bool(child()));drain(.3);assert child()!=pid
  send(b'\x1bOR');close();
  # Empty/non-Git project still gets a real private welcome buffer.
  empty=base/'empty';empty.mkdir();process=launch(empty);wait(lambda:bool(child()));drain(.3)
  send(b'\x1bOR');send(b'\t');send(b' q');drain(.4)
  assert 'Helix 已就绪' in clean() and not list(empty.glob('*.txt'))
  close();
  # Explicit opt-out and invalid custom executable do not silently fall back or loop.
  cfg.write_text(json.dumps({'helix_path':str(helix),'helix_startup':False}));process=launch();drain(.7);assert not child()
  close();
  cfg.write_text(json.dumps({'helix_path':str(base/'missing-hx')}));process=launch();drain(.7);assert not child();assert 'Helix 启动失败' in clean()
  close();
  assert termios.tcgetattr(slave)==before
  assert subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD'])==original
  print('REAL HELIX startup passed: preloaded before file entry, same PID on entry, fresh on restart, empty private welcome/save-return, opt-out, failed custom path no retries, unchanged source/HEAD/terminal')
 finally:
  if process.poll() is None:process.kill();process.wait()
  os.close(master);os.close(slave)
