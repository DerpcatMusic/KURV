#!/usr/bin/env python3
"""Compile exact baseline/candidate route methods without private plugin dependencies."""
import argparse, pathlib, subprocess, tempfile, json
ROOT = pathlib.Path(__file__).resolve().parents[3]
BASE = 'd084681411a95803bb52206647c2bc881c4cbf8b'
def method(text, name):
    start=text.index('    ', text.rfind('\n',0,text.index('fn '+name+'(') if 'fn '+name+'(' in text else text.index('fn '+name+'<'))+1)
    if text[:start].endswith('    #[inline(always)]\n'):
        start -= len('    #[inline(always)]\n')
    brace=text.index('{',start); depth=1; end=brace+1
    while depth:
        depth += (text[end]=='{')-(text[end]=='}'); end+=1
    return text[start:end].replace('pub(super) ', 'pub ')
def module(text, candidate):
    start=text.index('#[derive(Clone, Copy)]\nstruct GeneratorStructuralRoute {')
    end=text.index('\nimpl GeneratorStructuralRouteFrame {',start)
    defs=text[start:end].replace('pub(super) ', 'pub ')
    methods=['accumulate_phase_block','accumulate_pitch_block','block_amount']
    if candidate: methods+=['block_depth_amounts','mixed_gain_amount_block']
    return defs+'\nimpl GeneratorStructuralRouteFrame {\n'+ '\n'.join(method(text,n) for n in methods)+'''\n
    pub fn set_control(&mut self, control: crate::OscillatorControl) {
        for route in self.entries.iter_mut().flatten() { route.control=control; }
    }
    pub fn expected_gain<const N: usize>(&self,sources:&[[f32;N];32],over:Option<(u8,&[f32])>) -> [f32;N] {
        let route=self.entries[0].unwrap();
        std::array::from_fn(|i|self.block_amount(route,sources,i,over.map(|(t,a)|(t,a[i]))))
    }
    pub fn fixture(parents: usize, incoming: usize, seed: u32) -> Self {
        let mut s=Self::default();
        for r in 0..incoming {
            s.entries[r]=Some(GeneratorStructuralRoute {route_index:(r+8) as u8,source:r as u8,target:31,amount:if seed%3==0 {0.0} else {-0.73},control: crate::OscillatorControl::PhasePosition});
            s.next[r]=if r+1<incoming {(r+1) as u8} else {NO_GENERATOR_ROUTE};
            for d in 0..parents {
                let i=r*parents+d;
                s.depth_entries[i]=Some(GeneratorDepthRoute {source:(4+d%24) as u8,target_route:(r+8) as u8,amount:if seed%4==0 {0.0} else {0.31-(d as f32)*0.047}});
                s.depth_next[i]=if d+1<parents {(i+1) as u8} else {NO_GENERATOR_ROUTE};
            }
            if parents!=0 {s.depth_heads[r+8]=(r*parents) as u8;}
        }
        s.target_heads[31]=0;s
    }
}\n'''
RUST='''
#![allow(dead_code)]
use std::hint::black_box;
const EXTRA_MODULATION_ROUTE_COUNT:usize=48;
const MODULATION_ROUTE_COUNT:usize=64;
const MAX_OSCILLATORS:usize=32;
const MAX_AUX_MODULES:usize=32;
#[derive(Clone, Copy, PartialEq)] enum OscillatorControl {PhasePosition, Level, Transpose, Cents, Pan, RingModAmount}
#[derive(Clone, Copy)] enum FilterControl {Cutoff}
fn proof<const N:usize>() -> usize {
 let mut checked=0;
 for seed in 0..40_u32 {for parents in [0,1,2,4,8,16] {for incoming in [1,2,4] {
  let mut a=baseline::GeneratorStructuralRouteFrame::fixture(parents,incoming,seed);
  let mut b=candidate::GeneratorStructuralRouteFrame::fixture(parents,incoming,seed);
  let mut rng=seed+1;
  let sources: [[f32;N];32]=std::array::from_fn(|_| std::array::from_fn(|_| {rng=rng.wrapping_mul(1664525).wrapping_add(1013904223);(rng as i32 as f32)/(i32::MAX as f32)*2.0}));
  let amounts: [f32;N]=std::array::from_fn(|i| if seed%5==0 {0.0} else {((i as f32)*0.317).sin()*1.8});
  for target in [None,Some(8),Some(9),Some(63)] {
   let over=target.map(|t|(t,amounts.as_slice()));
   for control in [OscillatorControl::PhasePosition,OscillatorControl::Transpose,OscillatorControl::Cents] {
   a.set_control(control); b.set_control(control);
   let mut x=std::array::from_fn(|i| (i as f32)*0.013-0.73);let mut y=x;
   if control==OscillatorControl::PhasePosition {
   a.accumulate_phase_block(31,&sources,over,&mut x);b.accumulate_phase_block(31,&sources,over,&mut y);
   } else { a.accumulate_pitch_block(31,&sources,over,&mut x);b.accumulate_pitch_block(31,&sources,over,&mut y); }
   for i in 0..N {assert_eq!(x[i].to_bits(),y[i].to_bits(),"N={N} seed={seed} parents={parents} incoming={incoming} override={target:?} frame={i}");checked+=1;}
   }
   for control in [OscillatorControl::Level,OscillatorControl::Pan,OscillatorControl::RingModAmount] {
    a.set_control(control);b.set_control(control);
    let expected=a.expected_gain(&sources,over);let mut actual=[0.0;N];
    assert_eq!(b.mixed_gain_amount_block(31,&sources,over,&mut actual),Some(parents!=0 || target==Some(8)));
    for i in 0..N {assert_eq!(expected[i].to_bits(),actual[i].to_bits());checked+=1;}
   }
  }
 }}}
 checked
}
fn bench<const N:usize>() {
 let sources: [[f32;N];32]=std::array::from_fn(|s|std::array::from_fn(|i|((s*N+i) as f32*0.371).sin()));
 let amounts: [f32;N]=std::array::from_fn(|i|((i as f32)*0.317).sin());
 for parents in [0,1,2,4,8,16] {for incoming in [1,4] {for overridden in [false,true] {
 let a=baseline::GeneratorStructuralRouteFrame::fixture(parents,incoming,7);
 let b=candidate::GeneratorStructuralRouteFrame::fixture(parents,incoming,7);
 let mut output=[0.0;N];let iterations=30000;
 let over=overridden.then_some((8,amounts.as_slice()));
 for round in 0..8 {for which in if round%2==0 {[0,1]} else {[1,0]} {
 let start=std::time::Instant::now();
 for _ in 0..iterations {output.fill(0.0);if which==0 {black_box(&a).accumulate_phase_block(31,black_box(&sources),black_box(over),black_box(&mut output));}else{black_box(&b).accumulate_phase_block(31,black_box(&sources),black_box(over),black_box(&mut output));}black_box(&output);}
 println!("{N},{parents},{incoming},{overridden},{round},{which},{}",start.elapsed().as_nanos() as f64/iterations as f64);
 }}}}}
}
fn main(){if std::env::args().any(|a|a=="--bench") {println!("frames,parents,incoming,override,round,candidate,ns_block");bench::<32>();bench::<128>();}else{let count=proof::<1>()+proof::<8>()+proof::<32>()+proof::<128>();println!("{count} bit-identical sample comparisons passed");}}
'''
def main():
    p=argparse.ArgumentParser();p.add_argument('--bench',action='store_true');p.add_argument('--native',action='store_true');a=p.parse_args()
    old=subprocess.check_output(['git','show',BASE+':src/voices/poly_synth.rs'],cwd=ROOT,text=True)
    new=(ROOT/'src/voices/poly_synth.rs').read_text()
    source=RUST+'\nmod baseline {use super::*;'+module(old,False)+'}\nmod candidate {use super::*;'+module(new,True)+'}\n'
    with tempfile.TemporaryDirectory(prefix='kurv-depth-') as d:
        src=pathlib.Path(d)/'main.rs';src.write_text(source);binary=pathlib.Path(d)/'probe'
        rustc=pathlib.Path.home()/'.cargo/bin/rustc'
        cmd=[str(rustc),'+1.97.1','--edition=2024','-C','opt-level=3',str(src),'-o',str(binary)]
        if a.native:cmd+=['-C','target-cpu=native']
        subprocess.run(cmd,check=True)
        subprocess.run([str(binary)]+(['--bench'] if a.bench else []),check=True)
if __name__=='__main__':main()
