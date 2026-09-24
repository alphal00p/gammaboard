//! Four activity traces, averaged over reporting worker-time in bounded display bins.
use super::*;
use crate::server::panels::{PlotPoint, PlotSeries, PlotXAxis};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn sampler_rows_in_interval<'a>(
    rows: &'a [Value],
    interval: &Interval<'_>,
) -> Vec<&'a Value> {
    rows.iter()
        .filter(|row| {
            identity(row, false) == identity(interval.last, false)
                && timestamp(row) > timestamp(interval.first)
                && timestamp(row) <= timestamp(interval.last)
        })
        .collect()
}

fn identity(row: &Value, evaluator: bool) -> Option<(&str, &str, &str, &str)> {
    let metrics = data(row, evaluator);
    Some((
        row["worker_id"].as_str()?,
        metrics["node_uuid"].as_str()?,
        epoch(row, evaluator)?,
        metrics["task_id"].as_str()?,
    ))
}

fn series(id: &str, label: &str, color: &str) -> PlotSeries {
    PlotSeries {
        id: id.into(),
        label: label.into(),
        color: Some(color.into()),
        smooth: Some(false),
        points: Vec::new(),
    }
}

#[derive(Default)]
struct Bin {
    weighted: f64,
    seconds: f64,
    invalid: bool,
    identities: BTreeSet<usize>,
    ranges: Vec<[f64; 2]>,
    from: Option<f64>,
    to: f64,
}

// Intersect each measured interval with the bin; never fill unobserved time with zeros.
fn add_interval(
    bins: &mut [Bin],
    bounds: [f64; 2],
    from: f64,
    to: f64,
    percent: Option<f64>,
    measured_seconds: f64,
    group: usize,
) {
    let width = (bounds[1] - bounds[0]) / bins.len() as f64;
    let first = (((from - bounds[0]) / width).floor().max(0.0) as usize).min(bins.len());
    let last = (((to - bounds[0]) / width).ceil().max(0.0) as usize).min(bins.len());
    for (index, bin) in bins.iter_mut().enumerate().take(last).skip(first) {
        let start = from.max(bounds[0] + index as f64 * width);
        let end = to.min(bounds[0] + (index + 1) as f64 * width);
        if end <= start {
            continue;
        }
        bin.identities.insert(group);
        // Maintain the union, not one allocation per worker snapshot. Out-of-order
        // interval starts are possible when workers publish at different cadences.
        let mut range = [start, end];
        bin.ranges.retain(|existing| {
            if existing[1] + 1.0 < range[0] || existing[0] > range[1] + 1.0 {
                return true;
            }
            range[0] = range[0].min(existing[0]);
            range[1] = range[1].max(existing[1]);
            false
        });
        bin.ranges.push(range);
        bin.ranges.sort_by(|a, b| a[0].total_cmp(&b[0]));
        bin.from = Some(bin.from.map_or(start, |v| v.min(start)));
        bin.to = bin.to.max(end);
        if let Some(value) = percent {
            let weight = measured_seconds * (end - start) / (to - from);
            bin.weighted += value * weight;
            bin.seconds += weight;
        } else {
            bin.invalid = true;
        }
    }
}

fn finish(series: &mut PlotSeries, bins: Vec<Bin>) {
    let mut previous = BTreeSet::new();
    for bin in bins {
        if bin.invalid || bin.ranges.len() != 1 || bin.seconds <= 0.0 {
            previous.clear();
            continue;
        }
        let Some(from) = bin.from else {
            continue;
        };
        let gap = previous != bin.identities
            || series.points.last().is_none_or(|last| from - last.x > 1.0);
        let value = bin.weighted / bin.seconds;
        series.points.push(PlotPoint {
            x: from,
            y: value,
            break_before: gap.then_some(true),
            ..Default::default()
        });
        series.points.push(PlotPoint {
            x: bin.to,
            y: value,
            ..Default::default()
        });
        previous = bin.identities;
    }
}

pub(super) const DISPLAY_BINS: usize = 600;

pub(super) struct Graphs {
    bounds: [f64; 2],
    bins: [Vec<Bin>; 5],
    previous: BTreeMap<(bool, String), Value>,
    groups: BTreeMap<(bool, String), usize>,
    next_group: usize,
    pub(super) cadence: [Cadence; 2],
}

#[derive(Default, serde::Serialize)]
pub(super) struct Cadence {
    intervals: usize,
    total_seconds: f64,
}

impl Graphs {
    pub(super) fn new(bounds: [f64; 2]) -> Self {
        Self {
            bounds,
            bins: std::array::from_fn(|_| (0..DISPLAY_BINS).map(|_| Bin::default()).collect()),
            previous: BTreeMap::new(),
            groups: BTreeMap::new(),
            next_group: 0,
            cadence: Default::default(),
        }
    }

    // Feed each role in timestamp order, including one snapshot on either side
    // of the viewport. Memory is bounded by display bins and worker count.
    pub(super) fn observe(&mut self, row: Value, evaluator: bool) {
        let Some(worker) = row["worker_id"].as_str() else {
            return;
        };
        let key = (evaluator, worker.to_owned());
        let Some(previous) = self.previous.insert(key.clone(), row.clone()) else {
            return;
        };
        if identity(&previous, evaluator).is_none()
            || identity(&previous, evaluator) != identity(&row, evaluator)
        {
            self.groups.remove(&key);
            return;
        }
        let group = *self.groups.entry(key).or_insert_with(|| {
            self.next_group += 1;
            self.next_group
        });
        let (Some(from), Some(to)) = (timestamp(&previous), timestamp(&row)) else {
            return;
        };
        let start = from.timestamp_millis() as f64;
        let end = to.timestamp_millis() as f64;
        let seconds = (end - start) / 1000.0;
        if seconds <= 0.0 || end <= self.bounds[0] || start >= self.bounds[1] {
            return;
        }
        let interval = Interval {
            first: &previous,
            last: &row,
            seconds,
            evaluator,
        };
        let count = interval.delta(if evaluator {
            "/samples_evaluated"
        } else {
            "/completed_samples_total"
        });
        let cadence = &mut self.cadence[usize::from(!evaluator)];
        cadence.intervals += 1;
        cadence.total_seconds += seconds;
        for (lane_index, lane) in ["compute", "io"].into_iter().enumerate() {
            add_interval(
                &mut self.bins[usize::from(!evaluator) * 2 + lane_index],
                self.bounds,
                start,
                end,
                count.and_then(|_| interval.busy_percent(lane)),
                interval.busy_seconds().unwrap_or(seconds),
                group,
            );
        }
        if !evaluator {
            add_interval(
                &mut self.bins[4],
                self.bounds,
                start,
                end,
                count.map(|v| v / seconds),
                seconds,
                group,
            );
        }
    }

    pub(super) fn panels(self) -> (Vec<PanelSpec>, Vec<PanelState>) {
        let mut panels = Vec::new();
        let mut states = Vec::new();
        let bounds = self.bounds;
        let [ec, ei, sc, si, rates] = self.bins;
        let mut busy = Vec::new();
        for (id, label, color, bins) in [
            ("evaluator-compute", "Evaluator compute busy", "#005f73", ec),
            ("evaluator-io", "Evaluator I/O active", "#ee9b00", ei),
            ("sampler-compute", "Sampler compute busy", "#7c3aed", sc),
            ("sampler-io", "Sampler I/O active", "#e76f51", si),
        ] {
            let mut line = series(id, label, color);
            finish(&mut line, bins);
            busy.push(line);
        }
        let mut rate = series("accepted-rate", "Accepted samples / s", "#0a9396");
        finish(&mut rate, rates);
        for (id, label, series, percent) in [
            ("busy_history", "Compute and I/O activity (%)", busy, true),
            (
                "accepted_rate_history",
                "Accepted samples / s",
                vec![rate],
                false,
            ),
        ] {
            add_panel(
                &mut panels,
                &mut states,
                id,
                label,
                PanelKind::MultiTimeseries,
                PanelState::MultiTimeseries {
                    panel_id: id.into(),
                    x_axis: PlotXAxis::WallTime,
                    series,
                    x_range: Some(bounds),
                    y_range: percent.then_some([0.0, 100.0]),
                },
            );
        }
        (panels, states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_binning_preserves_a_gap_inside_one_display_bin() {
        let mut bins = vec![Bin::default(), Bin::default()];
        add_interval(&mut bins, [0.0, 2000.0], 0.0, 300.0, Some(50.0), 0.3, 0);
        add_interval(&mut bins, [0.0, 2000.0], 700.0, 2000.0, Some(0.0), 1.3, 1);
        let mut line = series("test", "test", "#000000");
        finish(&mut line, bins);
        assert_eq!(line.points.len(), 2);
        assert_eq!(line.points[0].x, 1000.0);
        assert_eq!(line.points[0].y, 0.0);
        assert_eq!(line.points[0].break_before, Some(true));
    }
    #[test]
    fn overlapping_worker_intervals_fill_gaps_without_inventing_zeros() {
        let mut bins = vec![Bin::default()];
        add_interval(&mut bins, [0., 1000.], 0., 300., Some(50.), 0.3, 0);
        add_interval(&mut bins, [0., 1000.], 700., 1000., Some(50.), 0.3, 0);
        add_interval(&mut bins, [0., 1000.], 0., 1000., Some(100.), 1., 1);
        let mut line = series("test", "test", "#000");
        finish(&mut line, bins);
        assert_eq!(line.points.len(), 2);
        assert!((line.points[0].y - 81.25).abs() < 1e-9);
    }
    fn row(time: i64, elapsed: f64, compute: f64, epoch: &str) -> Value {
        json!({"worker_id":"worker", "created_at":DateTime::from_timestamp_millis(time).unwrap(),
            "metrics":{"epoch":epoch,"node_uuid":"node","task_id":"task", "samples_evaluated":elapsed,
                "busy":{"elapsed_seconds":elapsed,"compute_seconds":compute,"io_seconds":0.0}}})
    }

    #[test]
    fn zoom_uses_boundary_intervals_and_epochs_never_bridge() {
        let mut zoomed = Graphs::new([200., 300.]);
        zoomed.observe(row(0, 0., 0., "one"), true);
        zoomed.observe(row(1000, 1., 0.8, "one"), true);
        let (_, panels) = zoomed.panels();
        let PanelState::MultiTimeseries { series, .. } = &panels[0] else {
            panic!()
        };
        assert_eq!(series[0].points.len(), 2 * DISPLAY_BINS);
        assert!((series[0].points[0].y - 80.).abs() < 1e-9);
        assert!(
            series[0]
                .points
                .iter()
                .all(|p| p.x >= 200. && p.x <= 300. && p.y_min.is_none() && p.y_max.is_none())
        );

        let mut restarted = Graphs::new([0., 2000.]);
        restarted.observe(row(0, 0., 0., "one"), true);
        restarted.observe(row(1000, 1., 0.8, "two"), true);
        let (_, panels) = restarted.panels();
        let PanelState::MultiTimeseries { series, .. } = &panels[0] else {
            panic!()
        };
        assert!(series[0].points.is_empty());
    }
}
