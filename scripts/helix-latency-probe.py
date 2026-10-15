#!/usr/bin/env python3
"""Local PTY key-to-glyph latency; real Helix, temporary files, no models/network.
Defaults: 6000-line text, 120x32. --source copies a UTF-8 Rust file (<=1 MiB)
with a leading comment, then inserts inside that comment without invalidating code.
--rust-syntax full compares upstream processing against Duet's default fast profile.
This is stdout feedback, not SSH/client end-to-end or file-switch latency.
"""
import argparse,os,pty,fcntl,termios,struct,subprocess,tempfile,pathlib,select,time,statistics,json
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('hx',type=pathlib.Path);parser.add_argument('binaries',nargs='*',type=pathlib.Path)
parser.add_argument('--source',type=pathlib.Path);parser.add_argument('--rows',type=int,default=32);parser.add_argument('--cols',type=int,default=120)
parser.add_argument('--rust-syntax',choices=['fast','full'],default='fast')
a=parser.parse_args();hx=a.hx.resolve()
assert 10<=a.rows<=200 and 40<=a.cols<=400
body=None
if a.source:
 assert a.source.stat().st_size<=1024*1024
 body='// latency probe\n'+a.source.read_text()
def probe(binary=None):
 with tempfile.TemporaryDirectory(prefix='duet-hx-latency-') as d:
  root=pathlib.Path(d);file=root/('probe.rs' if body is not None else 'probe.txt')
  file.write_text(body if body is not None else ''.join(f'line {i:05} value alpha\n' for i in range(6000)))
  cfg=root/'zz-benchmark-config.json';cfg.write_text(json.dumps({'helix_rust_syntax':a.rust_syntax}))
  for args in [('init','-q'),('add','.'),('-c','user.name=Probe','-c','user.email=probe@example.com','commit','-qm','fixture')]:subprocess.run(['git','-C',d,*args],check=True,stdout=subprocess.DEVNULL)
  master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',a.rows,a.cols,0,0))
  args=[str(binary),'--helix',str(hx),'--config',str(cfg),'open',d] if binary else [str(hx),str(file)]
  p=subprocess.Popen(args,stdin=slave,stdout=slave,stderr=slave,env={**os.environ,'TERM':'xterm-256color','XDG_CONFIG_HOME':str(root/'config'),'XDG_CACHE_HOME':str(root/'cache')})
  received=0
  def drain(seconds,marker=None):
   nonlocal received
   deadline=time.perf_counter()+seconds
   while time.perf_counter()<deadline:
    if select.select([master],[],[],max(0,min(.01,deadline-time.perf_counter())))[0]:
     data=os.read(master,65536);received+=len(data)
     if b'\x1b[6n' in data:os.write(master,b'\x1b[1;1R')
     if b'\x1b[c' in data or b'\x1b[0c' in data:os.write(master,b'\x1b[?1;2c')
     if marker and marker in data:return time.perf_counter()
   if marker:raise RuntimeError('glyph response timeout')
  def cpu():
   s=pathlib.Path(f'/proc/{p.pid}/stat').read_text().split(') ',1)[1].split();return(int(s[11])+int(s[12]))/os.sysconf('SC_CLK_TCK')
  try:
   drain(1)
   if binary:os.write(master,b'\x1bOR');drain(1);os.write(master,b'\t');drain(.3)
   startcpu=cpu();start=time.perf_counter();drain(1);idle=100*(cpu()-startcpu)/(time.perf_counter()-start)
   os.write(master,b'gg2li' if body is not None else b'i');drain(.15);samples=[];traffic=[]
   for n in range(50):
    glyph=b'ZYXW'[n%4:n%4+1];drain(.02);before=received;t=time.perf_counter();os.write(master,glyph);samples.append(1000*(drain(2,glyph)-t));traffic.append(received-before)
   result={'binary':str(binary or hx),'fixture':'copied Rust source, comment insertion' if body is not None else '6000-line text','terminal':f'{a.cols}x{a.rows}','rust_syntax':a.rust_syntax if binary else 'upstream','samples':len(samples),'p50_ms':round(statistics.median(samples),2),'p95_ms':round(sorted(samples)[int(len(samples)*.95)-1],2),'max_ms':round(max(samples),2),'bytes_per_key_p50':statistics.median(traffic),'idle_host_cpu_percent':round(idle,2)}
   os.write(master,b'\x1b');drain(.1)
   if binary:
    os.write(master,b'\x1b[17~');drain(.4);os.write(master,b':editor-stop\r');drain(.1);os.write(master,b'\r');drain(.4);os.write(master,b'q')
   else:os.write(master,b':q!\r')
   drain(.3);p.wait(timeout=4);return result
  finally:
   if p.poll() is None:p.kill();p.wait()
   os.close(master);os.close(slave)
print(json.dumps(probe(),ensure_ascii=False),flush=True)
for b in a.binaries:print(json.dumps(probe(b.resolve()),ensure_ascii=False),flush=True)
