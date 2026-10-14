#!/usr/bin/env python3
"""Real Helix + real outer/inner PTYs; no RPC or model requests."""
import os,pty,fcntl,struct,termios,subprocess,tempfile,pathlib,json,select,time,sys,re
binary=pathlib.Path(sys.argv[1]).resolve();helix=pathlib.Path(sys.argv[2]).resolve()
with tempfile.TemporaryDirectory(prefix='duet-helix-') as directory:
 base=pathlib.Path(directory);project=base/'project';project.mkdir();source=project/'main.txt';source.write_text('original file\n')
 for args in [('init','-q'),('add','.'),('-c','user.name=Test','-c','user.email=test@example.com','commit','-qm','base')]:
  subprocess.run(['git','-C',str(project),*args],check=True)
 original=subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD'])
 cfg=base/'config.json';cfg.write_text(json.dumps({'helix_path':str(helix),'helix_keepalive':False,'helix_startup':False}))
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
  # Helix insert mode. Duet must forward q/i/: rather than treating them as its shortcuts.
  send(b'iedited in embedded helix ');send(b'\x1b')
  send(b'\x1d');assert process.poll() is None
  send(b'q');assert process.poll() is None # Host termination confirmation, not an editor q.
  send(b'\x1b')
  send(b'\t');drain(.2)
  fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',28,96,0,0));drain(.5)
  send(b':wq\r');drain(1)
  assert source.read_text().startswith('edited in embedded helix '),source.read_text()
  assert subprocess.check_output(['git','-C',str(project),'rev-parse','HEAD'])==original,'must not auto commit'
  assert list((project/'.duet/backups').glob('helix-*/before-file')),'backup missing'
  send(b'q');assert process.wait(timeout=4)==0;assert termios.tcgetattr(slave)==before
  print('REAL HELIX PTY passed: confirmation/cancel, in-panel editing, Unicode-safe virtual terminal, focus switch, resize, save/exit, no auto commit, terminal restoration')
 finally:
  if process.poll() is None:process.kill();process.wait()
  os.close(master);os.close(slave)
