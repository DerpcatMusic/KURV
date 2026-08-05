#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimatedValue {
    pub value: f32,
    pub target: f32,
    pub speed: f32,
}

impl AnimatedValue {
    pub const fn new(value: f32, speed: f32) -> Self {
        Self {
            value,
            target: value,
            speed,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn tick(&mut self, dt_seconds: f32) -> f32 {
        let dt = dt_seconds.clamp(0.0, 1.0);
        let alpha = 1.0 - (-self.speed.max(0.0) * dt).exp();
        self.value += (self.target - self.value) * alpha;
        self.value
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawerMotion {
    pub openness: AnimatedValue,
}

impl Default for DrawerMotion {
    fn default() -> Self {
        Self {
            openness: AnimatedValue::new(0.0, 18.0),
        }
    }
}

impl DrawerMotion {
    pub fn set_open(&mut self, open: bool) {
        self.openness.set_target(if open { 1.0 } else { 0.0 });
    }

    pub fn tick(&mut self, dt_seconds: f32) -> f32 {
        self.openness.tick(dt_seconds)
    }
}
