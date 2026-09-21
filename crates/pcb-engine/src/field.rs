use pcb_core::{Bounds, FieldView, Vec2};

pub trait FieldAlgorithm {
    fn name(&self) -> &'static str;
    fn configure(&mut self, bounds: Bounds, width: usize, height: usize);
    fn clear(&mut self);
    fn scatter(&mut self, point: Vec2, amount: f32);
    fn solve(&mut self, target_density: f32, smoothing_steps: usize);
    fn gradient(&self, point: Vec2) -> Vec2;
    fn max_pressure(&self) -> f32;
    fn cell_count(&self) -> usize;
    fn view(&self) -> Option<FieldView>;
}

#[derive(Clone, Debug, Default)]
pub struct EulerianDensityAlgorithm {
    field: Option<DensityField>,
}

impl FieldAlgorithm for EulerianDensityAlgorithm {
    fn name(&self) -> &'static str {
        "eulerian-density-v0"
    }

    fn configure(&mut self, bounds: Bounds, width: usize, height: usize) {
        let replace = self.field.as_ref().is_none_or(|field| {
            field.bounds != bounds || field.width != width || field.height != height
        });
        if replace {
            self.field = Some(DensityField::new(bounds, width, height));
        }
    }

    fn clear(&mut self) {
        self.field_mut().clear();
    }

    fn scatter(&mut self, point: Vec2, amount: f32) {
        self.field_mut().scatter(point, amount);
    }

    fn solve(&mut self, target_density: f32, smoothing_steps: usize) {
        self.field_mut()
            .solve_pressure(target_density, smoothing_steps);
    }

    fn gradient(&self, point: Vec2) -> Vec2 {
        self.field_ref().gradient(point)
    }

    fn max_pressure(&self) -> f32 {
        self.field_ref().max_pressure()
    }

    fn cell_count(&self) -> usize {
        let field = self.field_ref();
        field.width * field.height
    }

    fn view(&self) -> Option<FieldView> {
        Some(self.field_ref().view())
    }
}

impl EulerianDensityAlgorithm {
    fn field_ref(&self) -> &DensityField {
        self.field
            .as_ref()
            .expect("field algorithm must be configured before use")
    }

    fn field_mut(&mut self) -> &mut DensityField {
        self.field
            .as_mut()
            .expect("field algorithm must be configured before use")
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoField;

impl FieldAlgorithm for NoField {
    fn name(&self) -> &'static str {
        "disabled"
    }

    fn configure(&mut self, _bounds: Bounds, _width: usize, _height: usize) {}
    fn clear(&mut self) {}
    fn scatter(&mut self, _point: Vec2, _amount: f32) {}
    fn solve(&mut self, _target_density: f32, _smoothing_steps: usize) {}
    fn gradient(&self, _point: Vec2) -> Vec2 {
        Vec2::ZERO
    }
    fn max_pressure(&self) -> f32 {
        0.0
    }
    fn cell_count(&self) -> usize {
        0
    }
    fn view(&self) -> Option<FieldView> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct DensityField {
    pub bounds: Bounds,
    pub width: usize,
    pub height: usize,
    pub density: Vec<f32>,
    pub pressure: Vec<f32>,
    pub gradient_x: Vec<f32>,
    pub gradient_y: Vec<f32>,
}

impl DensityField {
    pub fn new(bounds: Bounds, width: usize, height: usize) -> Self {
        assert!(width >= 3 && height >= 3);
        let cells = width * height;
        Self {
            bounds,
            width,
            height,
            density: vec![0.0; cells],
            pressure: vec![0.0; cells],
            gradient_x: vec![0.0; cells],
            gradient_y: vec![0.0; cells],
        }
    }

    pub fn clear(&mut self) {
        self.density.fill(0.0);
        self.pressure.fill(0.0);
        self.gradient_x.fill(0.0);
        self.gradient_y.fill(0.0);
    }

    pub fn scatter(&mut self, point: Vec2, amount: f32) {
        let (x, y) = self.continuous_cell(point);
        let x0 = x.floor().clamp(0.0, (self.width - 1) as f32) as usize;
        let y0 = y.floor().clamp(0.0, (self.height - 1) as f32) as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = (x - x0 as f32).clamp(0.0, 1.0);
        let ty = (y - y0 as f32).clamp(0.0, 1.0);
        for (cx, cy, weight) in [
            (x0, y0, (1.0 - tx) * (1.0 - ty)),
            (x1, y0, tx * (1.0 - ty)),
            (x0, y1, (1.0 - tx) * ty),
            (x1, y1, tx * ty),
        ] {
            let index = self.index(cx, cy);
            self.density[index] += amount * weight;
        }
    }

    pub fn solve_pressure(&mut self, target_density: f32, smoothing_steps: usize) {
        for (pressure, density) in self.pressure.iter_mut().zip(&self.density) {
            *pressure = (*density - target_density).max(0.0);
        }
        let mut scratch = self.pressure.clone();
        for _ in 0..smoothing_steps {
            for y in 1..self.height - 1 {
                for x in 1..self.width - 1 {
                    let index = self.index(x, y);
                    scratch[index] = (self.pressure[index] * 4.0
                        + self.pressure[self.index(x - 1, y)]
                        + self.pressure[self.index(x + 1, y)]
                        + self.pressure[self.index(x, y - 1)]
                        + self.pressure[self.index(x, y + 1)])
                        / 8.0;
                }
            }
            std::mem::swap(&mut self.pressure, &mut scratch);
        }
        let cell = self.cell_size();
        for y in 1..self.height - 1 {
            for x in 1..self.width - 1 {
                let index = self.index(x, y);
                self.gradient_x[index] = (self.pressure[self.index(x + 1, y)]
                    - self.pressure[self.index(x - 1, y)])
                    / (2.0 * cell.x);
                self.gradient_y[index] = (self.pressure[self.index(x, y + 1)]
                    - self.pressure[self.index(x, y - 1)])
                    / (2.0 * cell.y);
            }
        }
    }

    pub fn gradient(&self, point: Vec2) -> Vec2 {
        let (x, y) = self.continuous_cell(point);
        let cx = x.round().clamp(0.0, (self.width - 1) as f32) as usize;
        let cy = y.round().clamp(0.0, (self.height - 1) as f32) as usize;
        let index = self.index(cx, cy);
        Vec2::new(self.gradient_x[index], self.gradient_y[index])
    }

    pub fn max_pressure(&self) -> f32 {
        self.pressure.iter().copied().fold(0.0, f32::max)
    }

    pub fn view(&self) -> FieldView {
        FieldView {
            width: self.width,
            height: self.height,
            values: self.pressure.clone(),
            max_value: self.max_pressure(),
        }
    }

    fn continuous_cell(&self, point: Vec2) -> (f32, f32) {
        let size = self.bounds.size();
        (
            (point.x - self.bounds.min.x) / size.x * (self.width - 1) as f32,
            (point.y - self.bounds.min.y) / size.y * (self.height - 1) as f32,
        )
    }

    fn cell_size(&self) -> Vec2 {
        let size = self.bounds.size();
        Vec2::new(
            size.x / (self.width - 1) as f32,
            size.y / (self.height - 1) as f32,
        )
    }

    fn index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }
}
