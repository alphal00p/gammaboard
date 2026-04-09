# TODO

## Platform
- [ ] implement `madnis` sampler-aggregator as a parametrization
- [ ] instrument and optimize `insert_batches` end to end, especially the `batch_inputs` write path
- [ ] add pdf to sampler, use it to plot integrand vs pdf in dashboard
- [ ] COPY BINARY` is PostgreSQL’s **bulk-load protocol**: instead of sending a huge SQL statement like `INSERT ... VALUES (...), (...), ...`, you stream rows in a compact binary format directly to Postgres.

## Dashboard
- [ ] extend image plots: complex Plotly image trace with phase-hue / magnitude-intensity legend
- [ ] import json of histograms and compare them to current.
