/// Damped spring used for every animated value in the notch.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring {
    pub fn new(value: f32, stiffness: f32, damping: f32) -> Self {
        Self { value, velocity: 0.0, target: value, stiffness, damping }
    }

    /// Advances the simulation; returns true while still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        let accel = self.stiffness * (self.target - self.value) - self.damping * self.velocity;
        self.velocity += accel * dt;
        self.value += self.velocity * dt;
        let settled = (self.target - self.value).abs() < 0.0005 && self.velocity.abs() < 0.005;
        if settled {
            self.value = self.target;
            self.velocity = 0.0;
        }
        !settled
    }

    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
