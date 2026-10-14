#!/usr/bin/env python3
"""Local PTY key-to-glyph latency; real Helix, no models/network or user files.
Usage: helix-latency-probe.py /path/to/hx [duet-binary ...]
Direct Helix and Duet use identical outer PTY dimensions. This is not an SSH benchmark.
"""
import os,pty,fcntl,termios,struct,subprocess,tempfile,pathlib,select,time,statistics,json,sys
hx=pathlib.Path(sys.argv[1]).resolve()
def probe(binary=None):
 with tempfile.TemporaryDirectory(prefix='duet-hx-latency-') as d:
  root=pathlib.Path(d);file=root/'probe.txt';file.write_text(''.join(f'line {i:05} value alpha\n' for i in range(6000)))
  for a in [('init','-q'),('add','.'),('-c','user.name=Probe','-c','user.email=probe@example.com','commit','-qm','fixture')]:subprocess.run(['git','-C',d,*a],check=True,stdout=subprocess.DEVNULL)
  master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',32,120,0,0))
  args=[str(binary),'--helix',str(hx),'open',d] if binary else [str(hx),str(file)]
  p=subprocess.Popen(args,stdin=slave,stdout=slave,stderr=slave,env={**os.environ,'TERM':'xterm-256color','XDG_CONFIG_HOME':str(root/'config'),'XDG_CACHE_HOME':str(root/'cache')})
  def drain(seconds,marker=None):
   deadline=time.perf_counter()+seconds;tail=b''
   while time.perf_counter()<deadline:
    if select.select([master],[],[],max(0,min(.01,deadline-time.perf_counter())))[0]:
     data=os.read(master,65536);tail=(tail+data)[-64:]
     if b'\x1b[6n' in data:os.write(master,b'\x1b[1;1R')
     if b'\x1b[c' in data or b'\x1b[0c' in data:os.write(master,b'\x1b[?1;2c')
     if marker and marker in data:return time.perf_counter()
   if marker:raise RuntimeError('glyph response timeout')
  def cpu():
   s=pathlib.Path(f'/proc/{p.pid}/stat').read_text().split(') ',1)[1].split();return(int(s[11])+int(s[12]))/os.sysconf('SC_CLK_TCK')
  try:
   drain(1)
   if binary:os.write(master,b'3');drain(1);os.write(master,b'\t');drain(.3)
   startcpu=cpu();start=time.perf_counter();drain(1);idle=100*(cpu()-startcpu)/(time.perf_counter()-start)
   os.write(master,b'i');drain(.15);samples=[]
   for n in range(50):
    glyph=b'ZYXW'[n%4:n%4+1];drain(.02);t=time.perf_counter();os.write(master,glyph);samples.append(1000*(drain(2,glyph)-t))
   result={'binary':str(binary or hx),'samples':len(samples),'p50_ms':round(statistics.median(samples),2),'p95_ms':round(sorted(samples)[int(len(samples)*.95)-1],2),'max_ms':round(max(samples),2),'idle_host_cpu_percent':round(idle,2)}
   # Close normally so no orphan editor or private worktree survives the temporary fixture.
   os.write(master,b'\x1b');drain(.1)
   if binary:
    os.write(master,b'\x1d');drain(.4);os.write(master,b':editor-stop\r');drain(.1);os.write(master,b'\r');drain(.4);os.write(master,b'q')
   else:os.write(master,b':q!\r')
   drain(.3);p.wait(timeout=4);return result
  finally:
   if p.poll() is None:p.kill();p.wait()
   os.close(master);os.close(slave)
print(json.dumps(probe(),ensure_ascii=False),flush=True)
for b in sys.argv[2:]:print(json.dumps(probe(pathlib.Path(b).resolve()),ensure_ascii=False),flush=True)
