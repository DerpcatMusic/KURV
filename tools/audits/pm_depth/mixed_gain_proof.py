#!/usr/bin/env python3
"""Execute actual new gain helper against the original mixed-render gain block."""
from pathlib import Path
import subprocess
import tempfile
import argparse
from probe import compiler_args, compiler_command

parser = argparse.ArgumentParser(description=__doc__)
compiler_args(parser)
args = parser.parse_args()

ROOT = Path(__file__).resolve().parents[3]
BASE = 'd084681411a95803bb52206647c2bc881c4cbf8b'
old = subprocess.check_output(['git', 'show', BASE + ':src/voices/voice/block_render.rs'], cwd=ROOT, text=True)
start = old.index('                let base_gain_position = ||')
end = old.index('            if generator_routes.source_mask()', start)
body = old[start:end].rsplit('            }', 1)[0]
body = body.replace('base_oscillator.left_gain', 'base.0').replace('base_oscillator.right_gain', 'base.1')
body = body.replace('oscillator_taps[source][frame]', 'source[frame]').replace('oscillator_audio[slot][frame]', 'output[frame]')
baseline = 'fn old<const SAMPLES:usize>(output:&mut[(f32,f32);SAMPLES],source:&[f32;SAMPLES],base:(f32,f32),control:OscillatorControl,amount:f32){' + body + '}'
new = (ROOT / 'src/voices/voice/mixed_gain.rs').read_text().replace('pub(super)', 'pub').replace('//!', '//')
driver = r'''
#![allow(dead_code,unreachable_patterns)]
#[derive(Clone,Copy)] enum OscillatorControl {Level,Pan,RingModAmount}
fn main(){
 let mut count=0; let mut maximum=0.0_f32;
 for control in [OscillatorControl::Level,OscillatorControl::Pan,OscillatorControl::RingModAmount] {
  for base in [(0.0,0.0),(0.0,0.8),(0.8,0.0),(0.3,0.9),(0.7,0.7)] {
   for amount in [-1.0_f32,-0.2,0.0,0.3,1.0] {
    let source=std::array::from_fn::<_,64,_>(|n| (n as f32*0.37).sin()*1.8);
    let initial=std::array::from_fn::<_,64,_>(|n|((n as f32*0.11).sin(),(n as f32*0.17).cos()));
    let mut a=initial;let mut b=initial;
    old(&mut a,&source,base,control,amount);
    candidate::apply(&mut b,&source,base,control,amount,&[amount;64],false);
    for n in 0..64 {assert_eq!(a[n].0.to_bits(),b[n].0.to_bits());assert_eq!(a[n].1.to_bits(),b[n].1.to_bits());count+=2;}
    let depths=std::array::from_fn::<_,64,_>(|n| (amount+(n as f32*0.31).sin()*1.3).clamp(-1.0,1.0));
    let mut actual=initial;
    candidate::apply(&mut actual,&source,base,control,amount,&depths,true);
    for n in 0..64 {
     let mut expected=[initial[n]];old(&mut expected,&[source[n]],base,control,depths[n]);
     for (x,y) in [(actual[n].0,expected[0].0),(actual[n].1,expected[0].1)] {
      assert!(x.is_finite());let error=(x-y).abs();maximum=maximum.max(error);
      assert!(error<0.000_001,"dynamic gain {x} vs {y}");count+=1;
     }
    }
   }
  }
 }
 println!("PASS {count} actual mixed-gain comparisons; static bit-identical, dynamic max error {maximum}");
}
'''
with tempfile.TemporaryDirectory(prefix='kurv-mixed-gain-') as directory:
    src = Path(directory) / 'main.rs'
    src.write_text(driver + baseline + '\nmod candidate {\n' + new + '\n}\n')
    binary = Path(directory) / 'proof'
    subprocess.run(compiler_command(args)+[ '--edition=2024', '-C', 'opt-level=3', str(src), '-o', str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
