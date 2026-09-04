#!/usr/bin/env python3
"""Continuous analytic PM -> raw waveform -> ideal lowpass, convergence checked."""
import csv, sys
from collections import defaultdict
import numpy as np
N=4096
cases=defaultdict(list)
with open(sys.argv[1]) as f:
    for row in csv.DictReader(f):
        key=(row['shape'],int(row['carrier_bin']),float(row['depth']),float(row['nested_depth']))
        cases[key].append((float(row['baseline']),float(row['candidate'])))
def oracle(key,factor):
    shape,carrier,depth,nested=key
    t=np.arange(N*factor,dtype=np.float64)/(N*factor)
    p=(carrier*t+0.123+depth*np.sin(2*np.pi*(193*t+nested*np.sin(2*np.pi*367*t))))%1
    raw=2*p-1 if shape=='saw' else 1-4*np.abs(p-.5)
    spectrum=np.fft.rfft(raw)
    kept=spectrum[:N//2+1].copy()/factor
    kept[-1]=0 # Strictly below Nyquist, consistent at every convergence factor.
    return np.fft.irfft(kept,n=N)
def db(e,r):
    power=np.mean(e*e)
    return -np.inf if power==0 else 10*np.log10(power/np.mean(r*r))
w=csv.writer(sys.stdout)
w.writerow(['shape','carrier_hz','pm_depth_cycles','nested_depth_cycles','reference_convergence_db','baseline_error_db','candidate_error_db','candidate_minus_baseline_db','reference_status'])
for key,values in cases.items():
    a=np.array(values)
    r64=oracle(key,256)
    r256=oracle(key,1024)
    resolution='converged-256x-1024x'
    if key[2]==0:
        phase=(np.arange(N)*key[1]/N+0.123)%1
        r256=np.zeros(N)
        for k in range(1,int(np.ceil(N/(2*key[1])))):
            if key[0]=='saw':r256+=-2/(np.pi*k)*np.sin(2*np.pi*k*phase)
            elif k%2:r256+=-8/(np.pi*np.pi*k*k)*np.cos(2*np.pi*k*phase)
        r64=r256.copy()
        resolution='exact-static-Fourier'

    old,new=db(a[:,0]-r256,r256),db(a[:,1]-r256,r256)
    w.writerow([key[0],key[1]*48000/N,key[2],key[3],round(db(r64-r256,r256),3),round(old,3),round(new,3),round(new-old,3),resolution+(' / candidate-below-convergence' if key[2] and new<db(r64-r256,r256) else '')])
