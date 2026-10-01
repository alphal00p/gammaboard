# Evaluator pacing removal experiment

Follow-up: [coalesced refill wakeups](../evaluator-wakeup/README.md) resolved the
short-generation regression and replaced evaluator pacing. This report preserves
the earlier direct-removal experiment and its negative result.

**Decision: do not land unconditional removal yet.** Removing the evaluator's
10 ms tick floor substantially improves small CPU batches, but repeatedly reduces
single-worker throughput with short generations. The experimental scheduling
patch was reverted; the existing evaluator setting and default remain. No commit
or push was made for this experiment. The earlier prefetch-restriction removal
is separate and is present in both compared binaries.

## Comparison

The candidate removes fixed pacing after productive evaluator work. When the
queue is empty, it sleeps until the existing retry deadline instead of spinning.
The existing exponential empty-claim backoff (2–100 ms, with jitter), database
error backoff, control polling, connection limits and sampler's 10 ms tick are
unchanged. Simply setting the old evaluator tick to zero would busy-spin between
empty-queue retry deadlines; that is not the candidate tested here.

| Workload | Evaluators | Samples/batch | Samples/generation | Median paired throughput change, feedback off / on | Pairs per mode |
| --- | ---: | ---: | ---: | --- | ---: |
| Native CPU work, approximately 5 µs/sample | 1 | 512 | 131,072 | **+238.24%** / not measured | 2 |
| Zero delay, short generations | 1 | 32,768 | 131,072 | **−5.94% / −10.48%** | 3 |
| Zero delay, default generation size | 1 | 32,768 | 1,048,576 | +5.32% / +5.79% | 1 |
| Zero delay, large fleet | 128 | 32,768 | 4,194,304 | +0.46% / +6.55% | 2 |
| Slow mock, 50 ms/sample | 256 | 16 | 8,192 | −0.75% / +0.20% | 2 |
| Five-second training updates, zero-delay evaluation | 128 | 16 | 4,096 | not measured / +0.33% | 1 |

The CPU case rose from median **46.1k to 156.0k samples/s**, with paired gains
of +234.90% and +241.58%. The short-generation regression occurred in every pair:
−0.70% to −6.74% without feedback, and −4.97% to −11.74% with feedback, including
reversed trial order. Its first observation used a 12-second window; two longer
repeats used 20 seconds. The default-generation check is one pair per mode and
supports a hypothesis, not a precise gain estimate.

At 128 evaluators, materialized throughput stayed near **5.5M samples/s**. The
training gains were −0.05% and +13.16%, so the +6.55% median should not be read as
a repeatable improvement. At 256 slow evaluators, both variants sustained about
**5.1k samples/s**, close to the nominal 256 / 0.05 = 5,120 samples/s. These slow
evaluators use sleeps with seeded 10% batch-duration jitter, not CPU work.

## Database and idle-worker pressure

Counters bracket each CLI observation. CPU numbers are average occupied cores,
not I/O-busy fractions. PostgreSQL had eight allowed physical cores throughout.

| Workload | Database CPU cores, paced → unpaced | Database transactions/s, paced → unpaced |
| --- | ---: | ---: |
| Small CPU batches | 0.18 → 0.36 | 512 → 976 |
| 128 fast evaluators, feedback off | 4.13 → 3.96 | 5,777 → 5,576 |
| 128 fast evaluators, feedback on | 4.18 → 4.37 | 5,700 → 5,493 |
| 256 slow evaluators, feedback off | 1.41 → 1.48 | 3,083 → 3,108 |
| 256 slow evaluators, feedback on | 1.41 → 1.43 | 3,080 → 3,079 |
| 128 evaluators with training pauses | 0.79 → 0.79 | 4,800 → 4,599 |

Small batches completed 3.38 times as much work while database CPU approximately
doubled, so CPU cost per accepted sample fell. WAL increased with useful work
(2.75 → 9.81 MB/s); removing pacing does not make that traffic free. The highest
database CPU observation was about 4.4 cores. No deadlocks or PostgreSQL temporary
file spills were observed, and all readiness, measurement and shutdown checks
passed. No overload symptoms appeared in these checks; they do not establish
unlimited capacity for arbitrary fleets or tiny batches.

During the training-pause case, aggregate evaluator CPU fell from 0.76 to 0.58
cores across 128 processes, with nearly identical accepted throughput. Waiting
on the retry deadline avoids CPU spinning. Removing empty-queue backoff as well
would be a different change and would allow repeated unsuccessful database polls;
it is not recommended or measured here.

## Likely mechanism and next step

In the regressing case each generation contains only four evaluator batches.
Without pacing, evaluators encounter empty queues more often, while the sampler
still refills on its 10 ms tick. The old tick floor also imposed an effective
minimum interval on empty polls; removing it lets unsuccessful retries advance
through the exponential backoff sooner. A later retry can then miss freshly
inserted work. The observed empty-queue ratios and increased exposed fetch waits
are consistent with this interaction: median exposed training fetch wait rose
from 2.94 to 4.50 ms/batch. The experiment does not isolate its exact
contribution from refill/insert timing.

Using the default generation size (32 batches per generation) changed the sign
of the comparison, supporting the refill/backoff explanation. It is not a reason
to hide the smaller-generation regression or require larger training windows.

The next focused experiment should decouple productive execution from idle
polling while preserving a sensible initial empty-queue retry cadence—roughly
the current effective 10 ms—followed by the existing capped backoff. Keep this
internal, without adding a tuning knob or concurrency. Repeat the short-generation
cases first, then the large-fleet and pause checks. If that does not resolve the
regression, profile sampler refill/insert completion and consider waking refill
work on completion instead of relying on the sampler tick. Do not delete the
productive tick setting until these comparisons hold up.

## Reproduction and limits

There were **38 valid measurements in 19 pairs**, taking 18.5 minutes across the
three studies, with clean shutdown for every private deployment. Each trial used
a fresh database and the same CPU sets:
five sampler cores with four I/O threads, eight database cores with 1 GiB shared
buffers, and 51 evaluator cores. Large fleets multiplex those evaluator cores.
CPU affinity does not reserve them against other users of this shared host.
The first matrix used 12–20-second windows, paired AB/BA order where repeated;
the two targeted follow-ups used 20-second windows. Single pairs have no reversed
repeat. Slow-work tests cover approximately 0.8-second batches, not arbitrarily
long evaluations. Feedback uses native sampler ingestion, not a real optimizer.

Both immutable executables used Rust 1.94 and the same optimized `dev-optim`
profile. PostgreSQL counters can lag publication; CPU/database diagnostics span
the CLI observation, which can differ from the accepted-work telemetry window.
These short trials are comparative measurements, not confidence intervals or
long-duration resource-exhaustion tests. Protocol benchmarks and historical
frontier/amortization plots were not relabeled with these results.

[All trials](results.json), [paired summaries](summary.json), and
[manifests](manifests.json) retain the measurements, CPU allocations, settings,
binary hashes, and hardware metadata. Raw windows, pressure counters, run cards,
experimental patch, immutable binaries, harnesses and restoration/build records
are in `/common/dev/cedric/setup-logs/evaluator-unpaced-20261001/`. Its
`run_study.py`, `single-worker-repeat/run_study.py`, and
`default-generation-check/run_study.py` are machine-specific drivers using the
existing benchmark lifecycle; `inputs/benchmarks/` preserves their shared helpers.
