"""FFT of exact Rust renders. Off-wanted-bin power is a lower bound on aliasing.
Wanted-bin complex error includes both response error and aliases landing there.
No time alignment, gain normalization, or postfilter is applied.
"""
import sys,json
from pathlib import Path
import numpy as np
N=65536
bins=[37,601,4001,10001,25001]
raw=np.fromfile(sys.argv[1],dtype='<f8').reshape(3,5,4,N)
rows=[]
for shape in range(3):
 for j,bin in enumerate(bins):
  phase=.173; step=bin/N; h=np.arange(1,int((N/2-1)//bin)+1)
  width=.5 if shape==1 else max(.31,step)
  coeff=(1j/(np.pi*h) if shape==0 else (1-np.exp(-2j*np.pi*h*width))/(1j*np.pi*h))*np.exp(2j*np.pi*h*phase)
  ideal=np.zeros(N//2+1,dtype=complex); ideal[h*bin]=coeff
  if shape:ideal[0]=2*width-1
  power=2*np.sum(abs(coeff)**2)
  mask=np.ones(N//2+1,dtype=bool);mask[0]=False;mask[h*bin]=False
  for method in range(4):
   y=raw[shape,j,method];s=np.fft.rfft(y)/N
   err=s-ideal
   def db(x):return float(10*np.log10(max(x,1e-30)/power))
   row=dict(shape=['saw','square','pulse'][shape],hz=bin*48000/N,method=['current','6sample','8sample','12sample'][method],total_db=db(2*np.sum(abs(err[1:])**2)),off_wanted_db=db(2*np.sum(abs(s[mask])**2)),wanted_complex_db=db(2*np.sum(abs(err[h*bin])**2)),dc_error=float(s[0].real-ideal[0].real),peak=float(max(abs(y))),fundamental_db=float(20*np.log10(abs(s[bin]/ideal[bin]))))
   rows.append(row)
Path(sys.argv[2]).write_text(json.dumps(rows,indent=2)+'\n')
print('shape,hz,current_total,k6_total,k8_total,k12_total,current_off,k6_off,k8_off,k12_off')
for i in range(0,len(rows),4):
 rr=rows[i:i+4]
 print(f'{rr[0]["shape"]},{rr[0]["hz"]:.2f},'+','.join(f'{r[k]:.2f}' for k in ['total_db','off_wanted_db'] for r in rr))
