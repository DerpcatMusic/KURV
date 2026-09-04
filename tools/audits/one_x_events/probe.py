"""Offline references oversample only the oracle, never the 1x renderer."""
import io,json,subprocess
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parent
BIN=ROOT/'target/release/kurv-one-x-events'
N=8192

def phase(c,n):
 t=n/N; tau=2*np.pi
 if c==0:return .123+83*t
 if c==1:return .123+1703*t
 if c==2:return .123+83*t+.22*np.sin(tau*997*t)
 if c==3:return .123+83*t+.22*np.sin(tau*997*t+1.4*np.sin(tau*211*t+1.1*np.sin(tau*53*t)))
 return .123-83*t

def ref(c,os,pl,kernel=False):
 t=np.arange(N*os)/os
 if pl:
  n=np.floor(t); a=phase(c,n); p=a+(phase(c,n+1)-a)*(t-n)
 else:p=phase(c,t)
 raw=2*(p-np.floor(p))-1
 spec=np.fft.rfft(raw)/(N*os)
 cut=spec[:N//2+1]*N; cut[-1]=0
 if kernel:
  x=np.arange(-16*os,16*os+1)/os
  h=.9*np.sinc(.9*x)*(.42+.5*np.cos(np.pi*x/16)+.08*np.cos(2*np.pi*x/16))
  taps=np.zeros(N*os); taps[np.arange(-16*os,16*os+1)%(N*os)]=h/h.sum()
  cut*=np.fft.rfft(taps)[:N//2+1]
 return np.fft.irfft(cut,n=N)

def db(a,b):return float(10*np.log10(np.mean((a-b)**2)/np.mean(b*b)))
results=[]
for c,name in enumerate(['static_486Hz','static_9979Hz','pm_reversals','nested_pm','reverse_static']):
 out=np.loadtxt(io.StringIO(subprocess.check_output([str(BIN),str(c)],text=True)),delimiter=',')
 analytic=ref(c,512,False); pl=ref(c,512,True)
 row={'case':name,'event_vs_analytic_db':db(out[:,0],analytic),'shipping_vs_analytic_db':db(out[:,1],analytic),'event_vs_linear_phase_db':db(out[:,0],pl),'shipping_vs_linear_phase_db':db(out[:,1],pl),'linear_reference_convergence_256_512_db':db(ref(c,256,True),pl),'analytic_reference_convergence_256_512_db':db(ref(c,256,False),analytic),'input_reconstruction_error_db':db(pl,analytic),'event_vs_linear_phase_same_filter_db':db(out[:,0],ref(c,512,True,True))}
 assert row['event_vs_analytic_db'] < row['shipping_vs_analytic_db'] - 4
 assert row['event_vs_linear_phase_same_filter_db'] < -50
 assert row['linear_reference_convergence_256_512_db'] < -50
 print(json.dumps(row),flush=True); results.append(row)
(ROOT/'results.json').write_text(json.dumps(results,indent=2)+'\n')
