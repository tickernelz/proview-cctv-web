use serde::{Deserialize, Serialize};

pub const MOUNT_CEILING: u8 = 0;
pub const MOUNT_TABLE: u8 = 1;
pub const MOUNT_WALL: u8 = 2;

pub const LENS_GENERAL: u8 = 0;
pub const LENS_VR360: u8 = 1;
pub const LENS_LVR360: u8 = 2;
pub const LENS_VR180: u8 = 3;
pub const LENS_DUAL_VR360: u8 = 4;
pub const LENS_DUAL_VR180: u8 = 5;
pub const LENS_VR180_PRO: u8 = 6;
pub const LENS_SINGLE_PIP: u8 = 7;

const PI: f32 = std::f32::consts::PI;
const TAU: f32 = 2.0 * PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MountType {
    Ceiling,
    Table,
    Wall,
}

impl MountType {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            MOUNT_CEILING => Some(Self::Ceiling),
            MOUNT_TABLE => Some(Self::Table),
            MOUNT_WALL => Some(Self::Wall),
            _ => None,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Ceiling => MOUNT_CEILING,
            Self::Table => MOUNT_TABLE,
            Self::Wall => MOUNT_WALL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LensType {
    General,
    Vr360,
    Lvr360,
    Vr180,
    DualVr360,
    DualVr180,
    Vr180Pro,
    SinglePip,
}

impl LensType {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            LENS_GENERAL => Some(Self::General),
            LENS_VR360 => Some(Self::Vr360),
            LENS_LVR360 => Some(Self::Lvr360),
            LENS_VR180 => Some(Self::Vr180),
            LENS_DUAL_VR360 => Some(Self::DualVr360),
            LENS_DUAL_VR180 => Some(Self::DualVr180),
            LENS_VR180_PRO => Some(Self::Vr180Pro),
            LENS_SINGLE_PIP => Some(Self::SinglePip),
            _ => None,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::General => LENS_GENERAL,
            Self::Vr360 => LENS_VR360,
            Self::Lvr360 => LENS_LVR360,
            Self::Vr180 => LENS_VR180,
            Self::DualVr360 => LENS_DUAL_VR360,
            Self::DualVr180 => LENS_DUAL_VR180,
            Self::Vr180Pro => LENS_VR180_PRO,
            Self::SinglePip => LENS_SINGLE_PIP,
        }
    }

    pub fn declares_dewarp(self) -> bool {
        !matches!(self, Self::SinglePip)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Calibration {
    pub lens_type: u8,
    pub center_offset_x: i16,
    pub center_offset_y: i16,
    pub radius: i16,
    pub image_width: i16,
    pub image_height: i16,
    pub view_angle: u8,
    pub view_mode: u8,
}

impl Calibration {
    pub fn lens(&self) -> Option<Lens> {
        if self.radius <= 0 || self.image_width <= 0 || self.image_height <= 0 {
            return None;
        }
        let center_x = self.image_width as f32 / 2.0 + self.center_offset_x as f32;
        let center_y = self.image_height as f32 / 2.0 + self.center_offset_y as f32;
        Lens::new(
            self.image_width as u32,
            self.image_height as u32,
            center_x,
            center_y,
            self.radius as f32,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Curve {
    Equidistant { max_theta_deg: f32 },
    Table { radii: Vec<f64>, step_deg: f64 },
}

impl Curve {
    pub fn equidistant(max_theta_deg: f32) -> Option<Self> {
        if !max_theta_deg.is_finite() || max_theta_deg <= 0.0 || max_theta_deg > 180.0 {
            return None;
        }
        Some(Self::Equidistant { max_theta_deg })
    }

    pub fn from_samples(samples: &[(f64, f64)]) -> Option<Self> {
        if samples.len() < 2 {
            return None;
        }
        let step_deg = samples[1].0 - samples[0].0;
        if step_deg.is_nan() || step_deg <= 0.0 {
            return None;
        }
        let mut radii = Vec::with_capacity(samples.len());
        for (index, (angle, radius)) in samples.iter().enumerate() {
            if index > 0 {
                let expected = samples[0].0 + step_deg * index as f64;
                if (*angle - expected).abs() > step_deg * 1.0e-6 {
                    return None;
                }
                if *radius <= radii[index - 1] {
                    return None;
                }
            }
            radii.push(*radius);
        }
        Some(Self::Table { radii, step_deg })
    }

    pub fn max_theta(&self) -> f32 {
        match self {
            Self::Equidistant { max_theta_deg } => max_theta_deg.to_radians(),
            Self::Table { radii, step_deg } => {
                ((radii.len() - 1) as f64 * step_deg).to_radians() as f32
            }
        }
    }

    fn theta_of_radius(&self, radius_px: f32, scale: f32) -> f32 {
        match self {
            Self::Equidistant { .. } => radius_px / scale,
            Self::Table { radii, step_deg } => {
                if !radius_px.is_finite() || radius_px <= 0.0 {
                    return 0.0;
                }
                let target = radius_px as f64;
                let last = radii[radii.len() - 1];
                if target >= last {
                    return ((radii.len() - 1) as f64 * step_deg).to_radians() as f32;
                }
                let upper = radii.partition_point(|entry| *entry <= target);
                let index = upper.saturating_sub(1).min(radii.len() - 2);
                let low = radii[index];
                let high = radii[index + 1];
                let frac = if high > low {
                    (target - low) / (high - low)
                } else {
                    0.0
                };
                ((index as f64 + frac) * step_deg).to_radians() as f32
            }
        }
    }

    fn radius_of_theta(&self, theta: f32, scale: f32) -> f32 {
        match self {
            Self::Equidistant { .. } => theta * scale,
            Self::Table { radii, step_deg } => {
                if !theta.is_finite() || theta <= 0.0 {
                    return 0.0;
                }
                let steps = theta as f64 * 180.0 / (PI as f64 * step_deg);
                let index = steps.floor() as usize;
                if index + 1 >= radii.len() {
                    return radii[radii.len() - 1] as f32;
                }
                let frac = steps - index as f64;
                (radii[index] + (radii[index + 1] - radii[index]) * frac) as f32
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Direction {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Direction {
    pub fn new(x: f32, y: f32, z: f32) -> Option<Self> {
        let length = (x * x + y * y + z * z).sqrt();
        if !length.is_finite() || length <= f32::EPSILON {
            return None;
        }
        Some(Self {
            x: x / length,
            y: y / length,
            z: z / length,
        })
    }

    pub fn theta(&self) -> f32 {
        self.z.clamp(-1.0, 1.0).acos()
    }

    pub fn azimuth(&self) -> f32 {
        self.y.atan2(self.x)
    }

    pub fn yaw_deg(&self) -> f32 {
        self.x.atan2(self.z).to_degrees()
    }

    pub fn pitch_deg(&self) -> f32 {
        self.y
            .atan2((self.x * self.x + self.z * self.z).sqrt())
            .to_degrees()
    }

    pub fn from_yaw_pitch_deg(yaw_deg: f32, pitch_deg: f32) -> Self {
        let (sin_yaw, cos_yaw) = yaw_deg.to_radians().sin_cos();
        let (sin_pitch, cos_pitch) = pitch_deg.to_radians().sin_cos();
        Self {
            x: cos_pitch * sin_yaw,
            y: sin_pitch,
            z: cos_pitch * cos_yaw,
        }
    }

    pub fn equirect(&self) -> (f32, f32) {
        let bearing = self.y.atan2(self.x);
        let elevation = self.z.atan2((self.x * self.x + self.y * self.y).sqrt());
        let u = bearing / TAU + 0.5;
        (u - u.floor(), 0.5 - elevation / PI)
    }

    pub fn from_equirect(u: f32, v: f32) -> Self {
        let bearing = (u - 0.5) * TAU;
        let elevation = (0.5 - v) * PI;
        let (sin_bearing, cos_bearing) = bearing.sin_cos();
        let (sin_elevation, cos_elevation) = elevation.sin_cos();
        Self {
            x: cos_elevation * cos_bearing,
            y: cos_elevation * sin_bearing,
            z: sin_elevation,
        }
    }

    pub fn rotated(&self, pan_deg: f32, tilt_deg: f32) -> Self {
        let (sin_pan, cos_pan) = pan_deg.to_radians().sin_cos();
        let (sin_tilt, cos_tilt) = tilt_deg.to_radians().sin_cos();
        let x = self.x * cos_pan + self.z * sin_pan;
        let z = -self.x * sin_pan + self.z * cos_pan;
        let y = self.y;
        Self {
            x,
            y: y * cos_tilt - z * sin_tilt,
            z: y * sin_tilt + z * cos_tilt,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lens {
    pub width: u32,
    pub height: u32,
    pub center_x: f32,
    pub center_y: f32,
    pub radius: f32,
    pub curve: Curve,
}

impl Lens {
    pub fn new(width: u32, height: u32, center_x: f32, center_y: f32, radius: f32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        if !center_x.is_finite() || !center_y.is_finite() || !radius.is_finite() {
            return None;
        }
        if radius <= 0.0 {
            return None;
        }
        Some(Self {
            width,
            height,
            center_x,
            center_y,
            radius,
            curve: Curve::Equidistant {
                max_theta_deg: 90.0,
            },
        })
    }

    pub fn with_curve(mut self, curve: Curve) -> Self {
        self.curve = curve;
        self
    }

    pub fn scale(&self) -> f32 {
        self.radius / self.curve.max_theta()
    }

    pub fn theta_of_radius(&self, radius_px: f32) -> f32 {
        self.curve.theta_of_radius(radius_px, self.scale())
    }

    pub fn radius_of_theta(&self, theta: f32) -> f32 {
        self.curve.radius_of_theta(theta, self.scale())
    }

    pub fn inside(&self, px: f32, py: f32) -> bool {
        px >= 0.0 && py >= 0.0 && px <= self.width as f32 - 1.0 && py <= self.height as f32 - 1.0
    }

    pub fn direction(&self, px: f32, py: f32) -> Direction {
        let dx = px - self.center_x;
        let dy = py - self.center_y;
        let radius = (dx * dx + dy * dy).sqrt();
        let theta = self.theta_of_radius(radius);
        let azimuth = dy.atan2(dx);
        let (sin_theta, cos_theta) = theta.sin_cos();
        let (sin_azimuth, cos_azimuth) = azimuth.sin_cos();
        Direction {
            x: sin_theta * cos_azimuth,
            y: sin_theta * sin_azimuth,
            z: cos_theta,
        }
    }

    pub fn pixel(&self, direction: Direction) -> Option<(f32, f32)> {
        let theta = direction.theta();
        if theta > self.curve.max_theta() + 1.0e-4 {
            return None;
        }
        let radius = self.radius_of_theta(theta);
        let azimuth = direction.azimuth();
        Some((
            self.center_x + radius * azimuth.cos(),
            self.center_y + radius * azimuth.sin(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub pan_deg: f32,
    pub tilt_deg: f32,
    pub fov_deg: f32,
    pub cam_z: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            pan_deg: 0.0,
            tilt_deg: 0.0,
            fov_deg: 90.0,
            cam_z: 0.0,
        }
    }
}

impl View {
    pub fn ray(&self, x: f32, y: f32) -> Direction {
        let squared = x * x + y * y;
        let inside = (1.0 + squared * (1.0 - self.cam_z)).max(0.0).sqrt();
        let z = (squared * self.cam_z - inside) / (squared + 1.0);
        let scale = self.cam_z - z;
        let local = Direction::new(scale * x, scale * y, -z).unwrap_or(Direction {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        });
        local.rotated(self.pan_deg, self.tilt_deg)
    }

    pub fn half_extent(&self) -> f32 {
        (self.fov_deg.clamp(1.0, 179.0) * 0.5).to_radians().tan()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Map {
    pub width: u32,
    pub height: u32,
    pub sources: Vec<Option<(f32, f32)>>,
}

impl Map {
    pub fn equirect(lens: &Lens, width: u32, height: u32) -> Self {
        let mut sources = Vec::with_capacity(width as usize * height as usize);
        for row in 0..height {
            let v = (row as f32 + 0.5) / height as f32;
            for column in 0..width {
                let u = (column as f32 + 0.5) / width as f32;
                let direction = Direction::from_equirect(u, v);
                sources.push(
                    lens.pixel(direction)
                        .filter(|(px, py)| lens.inside(*px, *py)),
                );
            }
        }
        Self {
            width,
            height,
            sources,
        }
    }

    pub fn ptz(lens: &Lens, width: u32, height: u32, view: &View) -> Self {
        let half = view.half_extent();
        let aspect = width as f32 / height.max(1) as f32;
        let mut sources = Vec::with_capacity(width as usize * height as usize);
        for row in 0..height {
            let y = (2.0 * (row as f32 + 0.5) / height as f32 - 1.0) * half;
            for column in 0..width {
                let x = (2.0 * (column as f32 + 0.5) / width as f32 - 1.0) * half * aspect;
                let direction = view.ray(x, y);
                sources.push(
                    lens.pixel(direction)
                        .filter(|(px, py)| lens.inside(*px, *py)),
                );
            }
        }
        Self {
            width,
            height,
            sources,
        }
    }

    pub fn source(&self, column: u32, row: u32) -> Option<(f32, f32)> {
        if column >= self.width || row >= self.height {
            return None;
        }
        self.sources[row as usize * self.width as usize + column as usize]
    }

    pub fn valid_count(&self) -> usize {
        self.sources.iter().filter(|entry| entry.is_some()).count()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbFrame {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        if data.len() != width as usize * height as usize * 3 {
            return None;
        }
        Some(Self {
            width,
            height,
            data,
        })
    }

    pub fn black(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 3],
        }
    }

    pub fn pixel(&self, column: u32, row: u32) -> [u8; 3] {
        let offset = (row as usize * self.width as usize + column as usize) * 3;
        [
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
        ]
    }

    pub fn sample(&self, x: f32, y: f32) -> [u8; 3] {
        let max_x = self.width as f32 - 1.0;
        let max_y = self.height as f32 - 1.0;
        let fx = x.clamp(0.0, max_x);
        let fy = y.clamp(0.0, max_y);
        let x0 = fx.floor() as u32;
        let y0 = fy.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let p00 = self.pixel(x0, y0);
        let p10 = self.pixel(x1, y0);
        let p01 = self.pixel(x0, y1);
        let p11 = self.pixel(x1, y1);
        let mut out = [0u8; 3];
        for channel in 0..3 {
            let top = p00[channel] as f32 * (1.0 - tx) + p10[channel] as f32 * tx;
            let bottom = p01[channel] as f32 * (1.0 - tx) + p11[channel] as f32 * tx;
            let value = top * (1.0 - ty) + bottom * ty;
            out[channel] = value.round().clamp(0.0, 255.0) as u8;
        }
        out
    }

    pub fn warp(&self, map: &Map) -> Self {
        let mut out = Self::black(map.width, map.height);
        for row in 0..map.height {
            for column in 0..map.width {
                if let Some((sx, sy)) = map.source(column, row) {
                    let color = self.sample(sx, sy);
                    let offset = (row as usize * map.width as usize + column as usize) * 3;
                    out.data[offset] = color[0];
                    out.data[offset + 1] = color[1];
                    out.data[offset + 2] = color[2];
                }
            }
        }
        out
    }

    pub fn from_ppm(bytes: &[u8]) -> Option<Self> {
        let mut fields: Vec<&[u8]> = Vec::new();
        let mut index = 0usize;
        while fields.len() < 4 {
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'#' {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                continue;
            }
            if index >= bytes.len() {
                return None;
            }
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            fields.push(&bytes[start..index]);
        }
        if fields[0] != b"P6" {
            return None;
        }
        let width: u32 = std::str::from_utf8(fields[1]).ok()?.parse().ok()?;
        let height: u32 = std::str::from_utf8(fields[2]).ok()?.parse().ok()?;
        let max: u32 = std::str::from_utf8(fields[3]).ok()?.parse().ok()?;
        if max != 255 {
            return None;
        }
        index += 1;
        let expected = width as usize * height as usize * 3;
        let payload = bytes.get(index..index.checked_add(expected)?)?;
        Self::new(width, height, payload.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens() -> Lens {
        Lens::new(512, 512, 256.0, 256.0, 240.0).expect("lens")
    }

    fn spoke_color(index: usize) -> [u8; 3] {
        let hue = index as f32 / 12.0;
        let sector = hue * 6.0;
        let base = sector.floor();
        let fraction = sector - base;
        let ascending = (fraction * 255.0).round() as u8;
        let descending = 255 - ascending;
        match base as u32 % 6 {
            0 => [255, ascending, 0],
            1 => [descending, 255, 0],
            2 => [0, 255, ascending],
            3 => [0, descending, 255],
            4 => [ascending, 0, 255],
            _ => [255, 0, descending],
        }
    }

    fn fixture() -> RgbFrame {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/fisheye/fisheye-grid-512.ppm"
        );
        let bytes = std::fs::read(path).expect("fixture bytes");
        RgbFrame::from_ppm(&bytes).expect("fixture decodes")
    }

    fn median(values: &mut [f32]) -> f32 {
        values.sort_by(|left, right| left.partial_cmp(right).expect("ordered"));
        values[values.len() / 2]
    }

    fn close(color: [u8; 3], reference: [u8; 3], tolerance: i32) -> bool {
        (color[0] as i32 - reference[0] as i32).abs() <= tolerance
            && (color[1] as i32 - reference[1] as i32).abs() <= tolerance
            && (color[2] as i32 - reference[2] as i32).abs() <= tolerance
    }

    #[test]
    fn equidistant_reaches_its_max_angle_at_the_rim() {
        let lens = lens();
        assert!((lens.theta_of_radius(240.0).to_degrees() - 90.0).abs() < 1.0e-4);
        assert!((lens.theta_of_radius(120.0).to_degrees() - 45.0).abs() < 1.0e-4);
        let wide = Lens::new(2048, 2048, 1024.0, 1024.0, 1000.0)
            .unwrap()
            .with_curve(Curve::equidistant(180.0).unwrap());
        assert!((wide.theta_of_radius(1000.0).to_degrees() - 180.0).abs() < 1.0e-4);
    }

    #[test]
    fn pixel_and_direction_round_trip_stays_under_half_a_pixel() {
        let lens = lens();
        let mut worst = 0.0f32;
        for step in 0..64 {
            let angle = step as f32 * TAU / 64.0;
            for radius_step in 1..24 {
                let radius = radius_step as f32 * 10.0;
                let px = lens.center_x + radius * angle.cos();
                let py = lens.center_y + radius * angle.sin();
                let direction = lens.direction(px, py);
                let (back_x, back_y) = lens.pixel(direction).expect("inside");
                worst = worst.max((back_x - px).abs()).max((back_y - py).abs());
            }
        }
        assert!(worst < 0.5, "worst round trip {worst}");
    }

    #[test]
    fn table_curve_reproduces_its_samples() {
        let samples: Vec<(f64, f64)> = (0..=90)
            .map(|step| {
                (
                    step as f64,
                    (step as f64).to_radians() * 240.0 / std::f64::consts::FRAC_PI_2,
                )
            })
            .collect();
        let curve = Curve::from_samples(&samples).expect("curve");
        let lens = lens().with_curve(curve);
        let mut worst = 0.0f32;
        for step in 0..90 {
            let angle = step as f32;
            let radius = lens.radius_of_theta(angle.to_radians());
            let back = lens.theta_of_radius(radius).to_degrees();
            worst = worst.max((back - angle).abs());
        }
        assert!(worst < 0.01, "worst table error {worst}");
    }

    #[test]
    fn spokes_share_one_equirect_column_across_their_radius() {
        let lens = lens();
        let mut worst = 0.0f32;
        for bearing_step in 0..12 {
            let bearing = bearing_step as f32 * 30.0;
            for radius_step in 4..24 {
                let radius = radius_step as f32 * 10.0;
                let px = lens.center_x + radius * bearing.to_radians().cos();
                let py = lens.center_y + radius * bearing.to_radians().sin();
                let (u, _) = lens.direction(px, py).equirect();
                let measured = (u - 0.5) * 360.0;
                let mut delta = (measured - bearing).abs() % 360.0;
                if delta > 180.0 {
                    delta = 360.0 - delta;
                }
                worst = worst.max(delta);
            }
        }
        assert!(worst < 0.01, "worst column drift {worst}");
    }

    #[test]
    fn view_centre_follows_pan_and_tilt() {
        let panned = View {
            pan_deg: 35.0,
            tilt_deg: 0.0,
            fov_deg: 70.0,
            cam_z: 0.0,
        };
        let centre = panned.ray(0.0, 0.0);
        assert!((centre.yaw_deg() - 35.0).abs() < 1.0e-3);
        assert!(centre.pitch_deg().abs() < 1.0e-3);

        let tilted = View {
            pan_deg: 0.0,
            tilt_deg: 15.0,
            fov_deg: 70.0,
            cam_z: 0.0,
        };
        let raised = tilted.ray(0.0, 0.0);
        assert!((raised.pitch_deg() + 15.0).abs() < 1.0e-3);
        assert!(raised.y < 0.0);
    }

    #[test]
    fn ptz_centre_samples_the_pan_direction() {
        let lens = lens();
        let view = View {
            pan_deg: 45.0,
            tilt_deg: 0.0,
            fov_deg: 60.0,
            cam_z: 0.0,
        };
        let map = Map::ptz(&lens, 320, 240, &view);
        let (px, py) = map.source(160, 120).expect("centre valid");
        assert!((px - 376.0).abs() < 1.5, "x {px}");
        assert!((py - 256.0).abs() < 1.0, "y {py}");
    }

    #[test]
    fn warp_places_the_fixture_spokes_and_rings() {
        let frame = fixture();
        let lens = lens();
        let map = Map::equirect(&lens, 720, 360);
        let pano = frame.warp(&map);
        assert_eq!(pano.width, 720);
        assert_eq!(pano.height, 360);
        assert!(
            map.valid_count() >= 720 * 360 / 2,
            "valid {}",
            map.valid_count()
        );

        let mut worst_bearing = 0.0f32;
        for index in 0..12 {
            let reference = spoke_color(index);
            let mut columns = Vec::new();
            for row in 40..165 {
                for column in 0..720 {
                    if close(pano.pixel(column, row), reference, 8) {
                        columns.push(column as f32);
                    }
                }
            }
            assert!(columns.len() > 60, "spoke {index} pixels {}", columns.len());
            let measured = (median(&mut columns) + 0.5) / 720.0 * 360.0 - 180.0;
            let expected = index as f32 * 30.0;
            let mut delta = (measured - expected).abs() % 360.0;
            if delta > 180.0 {
                delta = 360.0 - delta;
            }
            worst_bearing = worst_bearing.max(delta);
        }
        assert!(worst_bearing < 1.0, "worst bearing error {worst_bearing}");

        let mut worst_elevation = 0.0f32;
        for (index, gray) in [96u8, 112, 128, 144, 160, 255].iter().enumerate() {
            let reference = [*gray, *gray, *gray];
            let mut rows = Vec::new();
            for row in 0..360 {
                for column in 0..720 {
                    if close(pano.pixel(column, row), reference, 4) {
                        rows.push(row as f32);
                    }
                }
            }
            assert!(rows.len() > 60, "ring {index} pixels {}", rows.len());
            let elevation = (0.5 - (median(&mut rows) + 0.5) / 360.0) * 180.0;
            let expected = 90.0 - (index as f32 + 1.0) * 15.0;
            worst_elevation = worst_elevation.max((elevation - expected).abs());
        }
        assert!(
            worst_elevation < 1.0,
            "worst elevation error {worst_elevation}"
        );
    }

    #[test]
    fn ppm_header_round_trips_dimensions() {
        let frame = RgbFrame::new(2, 1, vec![1, 2, 3, 4, 5, 6]).expect("frame");
        let mut bytes = b"P6\n# comment\n2 1\n255\n".to_vec();
        bytes.extend_from_slice(&frame.data);
        let decoded = RgbFrame::from_ppm(&bytes).expect("decoded");
        assert_eq!(decoded, frame);
    }
}
