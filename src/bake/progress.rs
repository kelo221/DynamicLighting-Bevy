//! Bake progress and cancellation, shared between the baking thread and the app.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// The step a bake is in. Steps run in this order; some are skipped when unused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum BakeStage {
    Preparing = 0,
    /// Photon cubes for bounce lighting.
    PhotonCubes = 1,
    /// Shadow rays, bounce, optimization, seams and serialization, counted in meshes.
    Meshes = 2,
    DistanceCubes = 3,
    Done = 4,
}

impl BakeStage {
    const ALL: [Self; 5] = [
        Self::Preparing,
        Self::PhotonCubes,
        Self::Meshes,
        Self::DistanceCubes,
        Self::Done,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::PhotonCubes => "photon cubes",
            Self::Meshes => "meshes",
            Self::DistanceCubes => "distance cubes",
            Self::Done => "done",
        }
    }
}

/// Lock-free progress counters plus a cancel flag. Share it through an `Arc`; the baker updates
/// it and checks for cancellation between work items.
#[derive(Debug, Default)]
pub struct BakeProgress {
    stage: AtomicU32,
    done: AtomicU64,
    total: AtomicU64,
    cancelled: AtomicBool,
}

impl BakeProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stage(&self) -> BakeStage {
        let stage = self.stage.load(Ordering::Relaxed) as usize;
        BakeStage::ALL
            .get(stage)
            .copied()
            .unwrap_or(BakeStage::Done)
    }

    /// Items finished in the current stage.
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    /// Items in the current stage.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Progress through the current stage, 0 to 1; 1 once the bake is done.
    pub fn fraction(&self) -> f32 {
        let total = self.total();
        if self.stage() == BakeStage::Done {
            1.0
        } else if total == 0 {
            0.0
        } else {
            (self.done() as f64 / total as f64).min(1.0) as f32
        }
    }

    /// Asks the bake to stop; it returns [`super::BakeError::Cancelled`] soon after.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub(crate) fn begin(&self, stage: BakeStage, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.stage.store(stage as u32, Ordering::Relaxed);
    }

    pub(crate) fn advance(&self, items: u64) {
        self.done.fetch_add(items, Ordering::Relaxed);
    }

    /// `Err(Cancelled)` once [`Self::cancel`] was called.
    pub(crate) fn check(&self) -> Result<(), super::BakeError> {
        if self.is_cancelled() {
            Err(super::BakeError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_cancels() {
        let progress = BakeProgress::new();
        assert_eq!(progress.stage(), BakeStage::Preparing);
        assert_eq!(progress.fraction(), 0.0);
        progress.begin(BakeStage::Meshes, 4);
        progress.advance(1);
        assert_eq!(progress.stage(), BakeStage::Meshes);
        assert_eq!((progress.done(), progress.total()), (1, 4));
        assert_eq!(progress.fraction(), 0.25);
        assert!(progress.check().is_ok());
        progress.cancel();
        assert!(progress.is_cancelled());
        assert_eq!(progress.check(), Err(super::super::BakeError::Cancelled));
        assert_eq!(BakeStage::DistanceCubes.name(), "distance cubes");
    }
}
