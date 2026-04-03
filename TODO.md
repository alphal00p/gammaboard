# TODO

## Platform
- [ ] implement `madnis` sampler-aggregator as a parametrization
- [ ] instrument and optimize `insert_batches` end to end, especially the `batch_inputs` write path
- [ ] PYo3 wrapper for generic python based integrand
- [ ] PYo3 wrapper for generic python based sampler
- [ ] add pdf to sampler, use it to plot integrand vs pdf in dashboard
- [ ] let the user save tasks and run tomls.

## Dashboard
- [ ] adjustable ranges for all plots
- [ ] better heatmap with plotly, asjustable colorscale (log / lin, sym / minmax)
- [ ] export svn/json/whatever of all plots buttons
- [ ] Reorder plots, e.g. progress more at top, tasks also at top, then right below that the live averages.
- [ ] import json of histograms and compare them to current.
