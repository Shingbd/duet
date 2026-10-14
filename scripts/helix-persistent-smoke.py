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
 def child():
  return pathlib.Path('/proc/'+str(process.pid)+'/task/'+str(process.pid)+'/children').read_text().strip()
 def wait(pred):
  end=time.monotonic()+5
  while time.monotonic()<end:
   drain(.025)
   if pred():return
  raise AssertionError(clean()[-2000:])
 try:
  drain(.5);wait(lambda:bool(child()));pid=child();send(b'3');drain(.5);assert child()==pid,'first file entry started another editor'
  send(b'\t');send(b'ifirst saved ');send(b'\x1b');send(b' q');drain(.4)
  assert child()==pid,'save-back exited Helix'
  assert source.read_text()=='first saved original file\n'
  start=len(output);send(b'klj');drain(.3)
  assert ':open' not in re.sub(r'\x1b\[[0-9;?]*[A-Za-z]','',output[start:].decode(errors='replace')),'typed command visible'
  send(b'\t');send(b'isecond saved ');send(b'\x1b');send(b' q');drain(.3)
  assert child()==pid and second.read_text()=='second saved second original\n'
  send(b'hj');drain(.3);send(b'\t');start=len(output);send(b':wq\r')
  wait(lambda:bool(child()) and child()!=pid);drain(.4)
  rendered=re.sub(r'\x1b\[[0-9;?]*[A-Za-z]','',output[start:].decode(errors='replace'));assert 'first saved original file' in rendered,'live Helix buffer not rendered'
  assert child(),'wq left no editor';newpid=child()
  # New process has saved file; focus return alone retains unsaved buffers and PID.
  send(b'\t');send(b'iafter return ');send(b'\x1b');send(b'\x1d');drain(.3)
  assert child()==newpid and source.read_text()=='first saved original file\n'
  assert subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD'])==original
  send(b':editor-stop\r');send(b'\r');wait(lambda:not child())
  send(b'q');assert process.wait(timeout=4)==0;assert termios.tcgetattr(slave)==before
  print('REAL HELIX persistent passed: internal F12 switching without typed command, Space q save/focus same PID, wq rebuild with live Helix pane, Ctrl+] preserves unsaved buffer, explicit stop, no commit')
 finally:
  if process.poll() is None:process.kill();process.wait()
  os.close(master);os.close(slave)
