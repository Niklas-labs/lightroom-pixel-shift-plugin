//! Sources expose normalized row windows while retaining their native storage.
use super::*;
#[derive(Clone, Debug)]
pub struct SensorInfo {
    pub width: usize,
    pub height: usize,
    pub cfa: [usize; 4],
    pub model: String,
    pub sequence: usize,
}
impl SensorInfo {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.width >= 8 && self.height >= 8 && self.width.checked_mul(self.height).is_some(),
            "invalid sensor dimensions"
        );
        let mut cfa = self.cfa;
        cfa.sort();
        ensure!(cfa == [0, 1, 1, 2], "only 2x2 RGB Bayer sensors supported");
        Ok(())
    }
    pub fn frame(&self, pixels: Vec<f32>, height: usize) -> Frame {
        Frame {
            width: self.width,
            height,
            pixels,
            cfa: self.cfa,
            model: self.model.clone(),
            sequence: self.sequence,
        }
    }
}
/// Reads an even-origin, full-width window. Implementations return errors before
/// reconstruction; workers never perform per-pixel file I/O.
pub trait SensorSource: Sync {
    fn info(&self) -> SensorInfo;
    fn read_rows(&self, start: usize, count: usize) -> Result<Frame>;
}
impl SensorSource for Frame {
    fn info(&self) -> SensorInfo {
        SensorInfo {
            width: self.width,
            height: self.height,
            cfa: self.cfa,
            model: self.model.clone(),
            sequence: self.sequence,
        }
    }
    fn read_rows(&self, start: usize, count: usize) -> Result<Frame> {
        ensure!(
            start.is_multiple_of(2)
                && count > 0
                && start
                    .checked_add(count)
                    .is_some_and(|end| end <= self.height),
            "invalid sensor row window"
        );
        Ok(self.info().frame(
            self.pixels[start * self.width..(start + count) * self.width].to_vec(),
            count,
        ))
    }
}
