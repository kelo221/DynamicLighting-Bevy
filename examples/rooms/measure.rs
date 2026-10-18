//! `--frames N`: the frame times over N frames after the warm-up, the GPU time of each render
//! pass, and what portal culling kept per frame, logged as one `rooms:` line.

use std::collections::BTreeMap;

use bevy::diagnostic::DiagnosticsStore;
use bevy::platform::time::Instant;

/// This frame's numbers.
#[derive(Clone, Copy, Default)]
pub struct Frame {
    pub visits: u32,
    pub lights: u32,
    pub cubes: u32,
    pub assigned: u32,
    pub faces: u32,
    pub entries: u32,
    pub fog: u32,
}

impl Frame {
    fn values(self) -> [u32; 7] {
        [
            self.visits,
            self.lights,
            self.cubes,
            self.assigned,
            self.faces,
            self.entries,
            self.fog,
        ]
    }
}

#[derive(Default)]
pub struct Measurement {
    frame_ms: Vec<f64>,
    sums: [f64; 7],
    /// The last batch of GPU pass times counted, and how many batches were.
    gpu_batch: Option<Instant>,
    gpu_batches: u32,
    gpu_ms: BTreeMap<String, f64>,
}

impl Measurement {
    pub fn frames(&self) -> usize {
        self.frame_ms.len()
    }

    pub fn add(&mut self, frame_ms: f64, frame: Frame, diagnostics: &DiagnosticsStore) {
        self.frame_ms.push(frame_ms);
        for (sum, value) in self.sums.iter_mut().zip(frame.values()) {
            *sum += f64::from(value);
        }
        self.add_gpu(diagnostics);
    }

    /// Counts each batch of GPU pass times once. A pass that didn't run in a frame keeps its
    /// last value in the store, so only the values of the newest batch are added.
    fn add_gpu(&mut self, diagnostics: &DiagnosticsStore) {
        let spans = || {
            diagnostics.iter().filter_map(|diagnostic| {
                let path = diagnostic.path().as_str().strip_suffix("/elapsed_gpu")?;
                let name = path.strip_prefix("render/").unwrap_or(path);
                Some((name, diagnostic.measurement()?))
            })
        };
        let Some(batch) = spans().map(|(_, measurement)| measurement.time).max() else {
            return;
        };
        if self.gpu_batch.is_some_and(|last| last >= batch) {
            return;
        }
        self.gpu_batch = Some(batch);
        self.gpu_batches += 1;
        for (name, measurement) in spans().filter(|(_, measurement)| measurement.time == batch) {
            *self.gpu_ms.entry(name.to_owned()).or_default() += measurement.value;
        }
    }

    /// The `rooms:` line.
    pub fn summary(&self, portals: bool) -> String {
        let count = self.frame_ms.len().max(1) as f64;
        let [visits, lights, cubes, assigned, faces, entries, fog] =
            self.sums.map(|sum| sum / count);
        let mut sorted = self.frame_ms.clone();
        sorted.sort_by(f64::total_cmp);
        let at = |share: f64| {
            sorted
                .get(((sorted.len() as f64 - 1.0) * share).round() as usize)
                .copied()
                .unwrap_or(0.0)
        };
        let mut line = format!(
            "rooms: portals {}, mean {:.3} ms per frame over {} frames, median {:.3}, p95 {:.3}; \
             per frame: visits {visits:.1}, realtime lights {lights:.2}, cubes {cubes:.2}, cubes \
             assigned {assigned:.3}, faces {faces:.2}, cluster entries {entries:.0}, fog {fog:.2}",
            if portals { "on" } else { "off" },
            self.frame_ms.iter().sum::<f64>() / count,
            self.frame_ms.len(),
            at(0.5),
            at(0.95),
        );
        if self.gpu_batches > 0 {
            let batches = f64::from(self.gpu_batches);
            // Nested spans are inside their parent's time already.
            let total: f64 = self
                .gpu_ms
                .iter()
                .filter(|(name, _)| !name.contains('/'))
                .map(|(_, ms)| ms / batches)
                .sum();
            line += &format!("; gpu ms: total {total:.3}");
            for (name, ms) in &self.gpu_ms {
                if ms / batches > 0.005 {
                    line += &format!(", {name} {:.3}", ms / batches);
                }
            }
        }
        line
    }
}
