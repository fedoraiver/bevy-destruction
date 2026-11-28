use std::collections::{BTreeMap, HashMap};

use bevy::{
    diagnostic::{DiagnosticPath, DiagnosticsStore},
    platform::time::Instant,
    prelude::*,
};

use crate::ProbeOptions;

pub struct MetricsPlugin;

impl Plugin for MetricsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RunMetrics>()
            .add_systems(Last, collect_metrics);
    }
}

#[derive(Default)]
#[cfg_attr(feature = "acceptance-test", derive(Reflect))]
pub struct Samples {
    pub count: usize,
    pub total_ms: f64,
    pub max_ms: f64,
}

impl Samples {
    fn record(&mut self, value: f64) {
        self.count += 1;
        self.total_ms += value;
        self.max_ms = self.max_ms.max(value);
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"samples":self.count,"average_ms":self.total_ms / self.count.max(1) as f64,"max_ms":self.max_ms})
    }
}

#[derive(Resource, Default)]
#[cfg_attr(feature = "acceptance-test", derive(Reflect))]
#[cfg_attr(feature = "acceptance-test", reflect(Resource))]
pub struct RunMetrics {
    pub phase: u32,
    pub wall_frame: Samples,
    pub render: BTreeMap<String, Samples>,
}

impl RunMetrics {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"wall_frame":self.wall_frame.json(),"render_passes":self.render.iter().map(|(k,v)| (k.clone(),v.json())).collect::<BTreeMap<_,_>>()})
    }
}

fn collect_metrics(
    probe_options: Res<ProbeOptions>,
    diagnostics: Option<Res<DiagnosticsStore>>,
    mut metrics: ResMut<RunMetrics>,
    mut last_frame: Local<Option<Instant>>,
    mut last_measurements: Local<HashMap<DiagnosticPath, Instant>>,
) {
    let now = Instant::now();
    let previous = last_frame.replace(now);
    if !probe_options.profile || metrics.phase < 80 {
        return;
    }
    if let Some(previous) = previous {
        metrics
            .wall_frame
            .record(now.duration_since(previous).as_secs_f64() * 1000.0);
    }
    if let Some(diagnostics) = diagnostics {
        for diagnostic in diagnostics.iter().filter(|d| {
            d.path().as_str().ends_with("elapsed_gpu") || d.path().as_str().ends_with("elapsed_cpu")
        }) {
            if let Some(measurement) = diagnostic.measurement() {
                let path = diagnostic.path();
                if last_measurements.get(path) == Some(&measurement.time) {
                    continue;
                }
                last_measurements.insert(path.clone(), measurement.time);
                metrics
                    .render
                    .entry(path.as_str().to_owned())
                    .or_default()
                    .record(measurement.value);
            }
        }
    }
}
