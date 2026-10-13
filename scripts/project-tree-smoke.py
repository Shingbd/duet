#!/usr/bin/env python3
"""Real Helix project tree: nested paths, same PTY, unsaved multi-buffer retention and explicit save-all."""
import os,pty,fcntl,struct,termios,subprocess,tempfile,pathlib,json,select,time,sys,re
binary=pathlib.Path(sys.argv[1]).resolve();helix=pathlib.Path(sys.argv[2]).resolve()
with tempfile.TemporaryDirectory(prefix='duet-helix-') as directory:
 base=pathlib.Path(directory);project=base/'project';project.mkdir();source=project/'main.txt';source.write_text('original file\n');(project/'src').mkdir();second=project/'src/second file.txt';second.write_text('second original\n')
 for args in [('init','-q'),('add','.'),('-c','user.name=Test','-c','user.email=test@example.com','commit','-qm','base')]:
  subprocess.run(['git','-C',str(project),*args],check=True)
 original=subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD'])
 cfg=base/'config.json';cfg.write_text(json.dumps({'helix_path':str(helix)}))
 master,slave=pty.openpty();before=termios.tcgetattr(slave);fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,130,0,0))
 process=subprocess.Popen([str(binary),'--config',str(cfg),'open',str(project)],stdin=slave,stdout=slave,stderr=slave,env={**os.environ,'TERM':'xterm-256color','XDG_CONFIG_HOME':str(base/'global')})
 output=bytearray()
 def drain(t=.15):
  end=time.monotonic()+t
  while time.monotonic()<end:
   if select.select([master],[],[],.01)[0]:output.extend(os.read(master,65536))
 def send(data):os.write(master,data);drain(.25)
 def clean():return re.sub(r'\x1b\[[0-9;?]*[A-Za-z]','',output.decode(errors='replace'))
 try:
  drain(.5);send(b'3');drain(2);assert source.read_text()=='original file\n'
  assert 'HELIX' in clean();send(b'\t')
  children=pathlib.Path('/proc/'+str(process.pid)+'/task/'+str(process.pid)+'/children')
  editor_pid=children.read_text().strip()
  send(b'ifirst unsaved ');send(b'\x1b');send(b'\x1d')
  # Directory first ordering: main.txt -> src -> expand -> second file.
  send(b'klj\r');drain(.6)
  assert source.read_text()=='original file\n','switch must not save first buffer'
  assert children.read_text().strip()==editor_pid,'switch restarted editor'
  send(b'\t');send(b'isecond unsaved ');send(b'\x1b');send(b'\x1d')
  assert second.read_text()=='second original\n'
  # Collapse src from child, then open main; original unsaved buffer must still exist.
  send(b'hj\r');drain(.4);send(b'\t');send(b':wqa\r');drain(1)
  assert source.read_text()=='first unsaved original file\n',source.read_text()
  assert second.read_text()=='second unsaved second original\n',second.read_text()
  assert subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD'])==original,'must not auto commit'
  assert list((project/'.duet/backups').glob('helix-*/before-file')),'backup missing'
  send(b'q');assert process.wait(timeout=4)==0;assert termios.tcgetattr(slave)==before
  print('REAL HELIX TREE passed: nested directory expand/collapse, spaced filenames, same PTY, two unsaved buffers retained, explicit save-all, backups, unchanged HEAD, terminal restoration')
 finally:
  if process.poll() is None:process.kill();process.wait()
  os.close(master);os.close(slave)
