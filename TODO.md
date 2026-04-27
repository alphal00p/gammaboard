# TODO

## Platform
- [ ] implement `madnis` sampler-aggregator as a parametrization

## Imported From todoist.csv
- [ ] Somehow expose additional metadata about the discrete bin (e.g. upon hovering over it). Also make sure that the export of the histogram is not lossy (but I'm pretty sure it's not already).
- [ ] Add percentage next to main result incl. error
- [ ] Reorder sections and entries within section to something prettier looking and more organized
- [ ] Make sure significant errors and warnings are logged
- [x] Fix bias in busy ratio % due to inconsistent rolling window
- [ ] Investigate why Hwu and Json observable bundles exported don't match
- [ ] Default linear/log Y axis in histogram visualization based on what the histogram metadata specifies
- [x] Expose toml of the active run, like currently done for tasks
- [ ] Fix the latching bug on the performance plots where it would never un-latch
- [ ] Do not dynamically sort the list of workers when selecting them and add a minimal filter based on task it is active on or inactive.
- [ ] In max weight, separately log and report the max wgt of the integrand and of the sampler, but still capture max wgt based off product of the two.
- [ ] investigate and study in more details the outcome of the PDF vs Integrand comparison plots
- [ ] And if possible it'd be able to manually adjust from the UI the spread of the color spectrum around the central value of 1.0 (meaning perfect sampling).
- [ ] rename "oversampling" in headers to "sampling accuracy"
- [ ] Overlay 1D histograms of the 2D slice imager, and also PDF vs integrand for the 1D slice imager
- [ ] Fix the timing fraction bar so as to only show timing fractions that make sense to be cumulative
- [ ] Add the stability percentile plot, per stability level
