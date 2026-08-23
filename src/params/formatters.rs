use super::KurvParams;

impl KurvParams {
    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_shape(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["SINE", "TRIANGLE", "SAW", "PULSE"];
        let value = value.clamp(0.0, 3.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "shape position is clamped to the four canonical waveforms"
        )]
        let lower = value.floor() as usize;
        let blend = value.fract();
        if blend <= 0.001 || lower == 3 {
            NAMES[lower].to_owned()
        } else {
            format!(
                "{} → {} {:.0}%",
                NAMES[lower],
                NAMES[lower + 1],
                blend * 100.0
            )
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_unison_curve(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "EVEN".to_owned()
        } else if value < 0.0 {
            format!("EDGES {:.0}%", -value * 100.0)
        } else {
            format!("CENTER {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_semitones(&self, value: f64) -> String {
        format!("{value:.2} st")
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_swarm_rate(&self, value: f64) -> String {
        format!("{value:.2} Hz")
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_phase_position(&self, value: f64) -> String {
        format!("{:.0}°", value.clamp(0.0, 1.0) * 360.0)
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_swarm_mode(&self, value: f64) -> String {
        if value.round() >= 1.0 {
            "SINE".to_owned()
        } else {
            "NOISE".to_owned()
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_unison_alignment_mode(&self, value: f64) -> String {
        ["NOTE", "HARM", "ODD", "EVEN"][value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "phase-warp mode is clamped to four discrete labels"
    )]
    pub(super) fn format_phase_warp_mode(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["NONE", "PWM", "BEND", "HARM"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "LFO mode is clamped to four discrete labels"
    )]
    pub(super) fn format_lfo_mode(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "LFO shape is clamped to four discrete labels"
    )]
    pub(super) fn format_lfo_shape(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["CURVE", "RANDOM HOLD", "RANDOM SMOOTH", "TRANCE GATE"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "LFO rate mode is clamped to four discrete labels"
    )]
    pub(super) fn format_lfo_rate_mode(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["Hz", "ms", "BEAT", "KEY"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sync division is clamped to the fixed musical division table"
    )]
    pub(super) fn format_lfo_sync(&self, value: f64) -> String {
        const NAMES: [&str; 16] = [
            "1/64", "1/32T", "1/32", "1/16T", "1/16", "1/8T", "1/8", "1/4T", "1/4", "1/2T", "1/2",
            "1/1T", "1/1", "2/1", "4/1", "8/1",
        ];
        NAMES[value.round().clamp(0.0, 15.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "modulation source is clamped to off plus 64 stable source slots"
    )]
    pub(super) fn format_mod_source(&self, value: f64) -> String {
        let source = value.round().clamp(0.0, 64.0) as usize;
        if source == 0 {
            "OFF".to_owned()
        } else {
            format!("SOURCE {source}")
        }
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "modulation target is clamped to the fixed oscillator target bank"
    )]
    pub(super) fn format_mod_target(&self, value: f64) -> String {
        const NAMES: [&str; 22] = [
            "OFF",
            "O1 PITCH",
            "O1 SHAPE",
            "O1 PWM",
            "O1 WARP",
            "O1 LEVEL",
            "O1 PAN",
            "O2 PITCH",
            "O2 SHAPE",
            "O2 PWM",
            "O2 WARP",
            "O2 LEVEL",
            "O2 PAN",
            "O3 PITCH",
            "O3 SHAPE",
            "O3 PWM",
            "O3 WARP",
            "O3 LEVEL",
            "O3 PAN",
            "O1 DETUNE",
            "O2 DETUNE",
            "O3 DETUNE",
        ];
        NAMES[value.round().clamp(0.0, 21.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the discrete stereo layout is clamped to four labels"
    )]
    pub(super) fn format_stereo_pattern(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["SHAPE", "ALTERNATE", "SHAPE", "RANDOM"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_envelope_curve(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "LINEAR".to_owned()
        } else if value < 0.0 {
            format!("SLOW {:.0}%", -value * 100.0)
        } else {
            format!("FAST {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_envelope_curve_time(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "CENTER".to_owned()
        } else if value < 0.0 {
            format!("EARLY {:.0}%", -value * 100.0)
        } else {
            format!("LATE {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_unison_weight(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "EVEN".to_owned()
        } else if value < 0.0 {
            format!("CENTER {:.0}%", -value * 100.0)
        } else {
            format!("SIDES {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the oversampling factor is clamped to the four visible quality modes"
    )]
    pub(super) fn format_oversampling(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["ECO 1x", "NORMAL 2x", "HIGH 3x", "ULTRA 4x"];
        NAMES[value.round().clamp(1.0, 4.0) as usize - 1].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the antialiasing selector has exactly three labels"
    )]
    pub(super) fn format_antialiasing(&self, value: f64) -> String {
        let _ = value;
        "SPLINE 4PT".to_owned()
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_generator_engine(&self, value: f64) -> String {
        let _ = value;
        "SPLINE 4PT".to_owned()
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_signed_semitones(&self, value: f64) -> String {
        format!("{:+.0} st", value.round())
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_cents(&self, value: f64) -> String {
        format!("{value:+.1} ct")
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_octaves(&self, value: f64) -> String {
        format!("{:+.0} oct", value.round())
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_voice_mode(&self, value: f64) -> String {
        match value.round() as i32 {
            0 => "MONO".to_owned(),
            1 => "LEGATO".to_owned(),
            voices => format!("{voices} VOICES"),
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    pub(super) fn format_glide_time(&self, value: f64) -> String {
        if value <= 0.000_5 {
            "OFF".to_owned()
        } else if value < 1.0 {
            format!("{:.0} ms", value * 1_000.0)
        } else {
            format!("{value:.2} s")
        }
    }
}
