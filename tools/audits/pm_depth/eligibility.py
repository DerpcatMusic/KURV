#!/usr/bin/env python3
"""Check exact production topology/eligibility with feed-forward and feedback parents."""
import argparse, pathlib, subprocess, tempfile
from probe import ROOT, BASE, method

def module(text):
    start=text.index('#[derive(Clone, Copy)]\nstruct GeneratorStructuralRoute {')
    end=text.index('\nimpl GeneratorStructuralRouteFrame {',start)
    defs=text[start:end].replace('pub(super) ', 'pub ')
    methods=['finish','mixed_phase_gain_routes','gain_block_eligible','phase_block_eligible','pitch_block_eligible','block_class']
    return defs+'\nimpl GeneratorStructuralRouteFrame {\n'+'\n'.join(method(text,n) for n in methods)+'''\n
    pub fn check(candidate:bool) -> usize {
        let mut checked=0;
        for control in [crate::OscillatorControl::Level,crate::OscillatorControl::Pan,crate::OscillatorControl::RingModAmount] {
         for swapped in [false,true] {for parents in [0,1,4,16] {for parent_target in [8_u8,9] {for feedback in [false,true] {
          let mut s=Self::default();
          let phase=GeneratorStructuralRoute {route_index:8,source:0,target:31,amount:-0.42,control:crate::OscillatorControl::PhasePosition};
          let gain=GeneratorStructuralRoute {route_index:9,control,..phase};
          let entries=if swapped {[gain,phase]} else {[phase,gain]};
          s.entries[0]=Some(entries[0]);s.entries[1]=Some(entries[1]);s.len=2;s.source_mask=1;s.target_mask=1<<31;
          for d in 0..parents {
           let source=if feedback {31} else {(1+d) as u8};
           s.depth_entries[d]=Some(GeneratorDepthRoute {source,target_route:parent_target,amount:0.31});s.source_mask |=1<<source;
          }
          s.depth_len=parents as u8;s.finish();
          let eligible=(candidate || parents==0) && !(feedback && parents>0);
          assert_eq!(s.mixed_phase_gain_routes().is_some(),eligible);
          assert_eq!(s.block_class(true),if eligible {2} else {0});
          assert_eq!(s.block_class(false),0);
          if !feedback {
           let target=s.order.iter().position(|v|*v==31).unwrap();
           for source in 0..=parents {assert!(s.order.iter().position(|v|*v==source as u8).unwrap()<target);}
          }
          let mut rejected=s;rejected.feedback_routes=1;assert!(rejected.mixed_phase_gain_routes().is_none());
          let mut rejected=s;rejected.feedback_depth_routes=1;assert!(rejected.mixed_phase_gain_routes().is_none());
          let mut rejected=s;rejected.filter_len=1;assert!(rejected.mixed_phase_gain_routes().is_none());
          let mut rejected=s;rejected.aux_target_mask=1;assert!(rejected.mixed_phase_gain_routes().is_none());
          let mut rejected=s;rejected.source_mask|=1<<31;assert!(rejected.mixed_phase_gain_routes().is_none());
          let mut rejected=s;rejected.entries[1].as_mut().unwrap().source=2;assert!(rejected.mixed_phase_gain_routes().is_none());
          let mut rejected=s;rejected.entries[1].as_mut().unwrap().target=30;assert!(rejected.mixed_phase_gain_routes().is_none());
          checked+=1;
         }}}}
        }
        checked
    }
}\n'''

def main():
    p=argparse.ArgumentParser();p.add_argument('--candidate-root',type=pathlib.Path,default=ROOT);a=p.parse_args()
    old=subprocess.check_output(['git','show',BASE+':src/voices/poly_synth.rs'],cwd=ROOT,text=True)
    new=(a.candidate_root/'src/voices/poly_synth.rs').read_text()
    source='''#![allow(dead_code)]
const EXTRA_MODULATION_ROUTE_COUNT:usize=48;
const MODULATION_ROUTE_COUNT:usize=64;
const MAX_OSCILLATORS:usize=32;
const MAX_AUX_MODULES:usize=32;
#[derive(Clone, Copy, PartialEq)] enum OscillatorControl {PhasePosition,Level,Transpose,Cents,Pan,RingModAmount}
#[derive(Clone, Copy)] enum FilterControl {Cutoff}
fn main(){let count=baseline::GeneratorStructuralRouteFrame::check(false)+candidate::GeneratorStructuralRouteFrame::check(true);println!("{count} graph fixtures passed baseline/candidate eligibility and production topology checks");}
'''+ '\nmod baseline {use super::*;'+module(old)+'}\nmod candidate {use super::*;'+module(new)+'}\n'
    with tempfile.TemporaryDirectory(prefix='kurv-depth-eligibility-') as d:
        src=pathlib.Path(d)/'main.rs';src.write_text(source);binary=pathlib.Path(d)/'probe'
        subprocess.run([str(pathlib.Path.home()/'.cargo/bin/rustc'),'+1.97.1','--edition=2024','-C','opt-level=3',str(src),'-o',str(binary)],check=True)
        subprocess.run([str(binary)],check=True)
if __name__=='__main__':main()
