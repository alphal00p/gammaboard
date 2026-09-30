"""Offline frontier figures and an operator-facing report."""
import json

import benchmark_common as bench
from benchmark_frontier import adequate, choose, summarize


def cost_label(us):
    return f'{us:g} µs' if us < 1000 else f'{us/1000:g} ms'


def report(directory, plots=True):
    manifest = json.loads((directory/'manifest.json').read_text())
    if manifest.get('schema_version') != 5:
        raise ValueError('use the saved harness for older frontier artifacts')
    records = bench.load_results(directory)
    summary = summarize(records)
    rows = summary['frontier']
    coordinate = manifest.get('value_coordinate')
    values = (f'The evaluator returns x[{coordinate}], a continuous sample coordinate, so feedback varies with the seeded input. '
              'This avoids constant-value feedback; it does not guarantee incompressibility for arbitrary input distributions.'
              if coordinate is not None else
              'The evaluator returns constant one; its training feedback is unusually compressible.')
    jitter = manifest.get('timing_jitter',dict(relative_sigma=0.,seed=0))
    timing = (f'Batch durations use reproducible Gaussian jitter with {jitter["relative_sigma"]:.0%} relative standard deviation '
              f'and seed {jitter["seed"]}. Zero delay stays zero. '
              'The batch-duration cap applies to the nominal mean, not each random draw.'
              if jitter['relative_sigma'] else 'Batch delays are fixed; this study did not enable timing jitter.')
    design = ('This study uses the default sparse curves: a full zero-delay doubling curve and sparse delayed '
              'anchors, reusing selected batch and queue settings in fresh runs. It measures scaling at those '
              'settings; it does not re-optimize every point.' if manifest.get('design') == 'sparse' else
              'This study validates an explicit sparse list of configurations in fresh runs, reusing selected '
              'batch and queue settings. It measures scaling at those settings; it does not establish a fully '
              'optimized frontier at every count.' if manifest.get('validation_points') is not None else
              'Zero delay checks all configured worker doublings. Delayed curves retain sparse scaling anchors, '
              'more doublings around predicted saturation, and a high-count check. At each measured count the search '
              'grows batches and checks a smaller batch before judging scaling.')
    if plots:
        draw(directory, manifest, rows, records)
    lines = ['# GammaBoard throughput and scaling', '',
        f"{len(records)} recorded trials; {sum(r['valid'] and adequate(r) for r in records)} valid and adequately sampled. "
        f"Elapsed {manifest['elapsed_seconds']/60:.1f} minutes; status: {manifest['status']}.", '',
        f"{manifest['cpu_model']}. {manifest['scope']} Initial host load: {manifest['load_average']}.", '',
        'RNG uses a frozen uniform Havana grid and compact input seeds. Materialized sends six-dimensional samples '
        'and returns compact accumulation. Training additionally returns and ingests per-sample values. '
        'The training window is fixed at 10¹² samples: this measures feedback transport, not optimizer barriers or learning.', '',
        values, '',
        'Each evaluator sleeps once per batch with nominal duration batch size × the configured delay; arithmetic work is disabled. '
        'The zero-delay case provides an empirical transport/housekeeping reference. This is a GammaBoard coordination benchmark, not CPU scaling. '
        'Sample generation, materialization, serialization and accumulation still do real work. Compute busy includes simulated waiting.', '',
        timing, '',
        design+' Unmeasured settings are not inferred. '
        'The refill threshold is fixed at one pending batch per evaluator. Live transitions wait until pre-change generated work and warmup work are accepted. '
        'Selected candidates are validated in fresh runs. Historical same-run confirmations remain identified separately. '
        'These short measurements do not establish a confidence-bound estimate of a 5% frontier.', '',
        '| Mode | Delay/sample | Evaluators | Batch | Accepted samples/s | Ideal achieved | Evaluator batches/s | Evidence |',
        '| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |']
    selected = []
    for mode in manifest['suite']['modes']:
        for cost in sorted(manifest['suite']['eval_us']):
            group = [r for r in rows if r['point']['mode'] == mode and r['point']['eval_us'] == cost]
            if not group:
                lines.append(f'| {mode} | {cost_label(cost)} | — | — | — | — | — | not measured adequately |')
                continue
            candidates = ([r for r in group if r['fresh_run']] or
                          [r for r in group if r['confirmed']] or group)
            winner = choose(candidates)
            p = winner['point']; selected.append(winner)
            evidence = ('fresh run' if winner['fresh_run'] else
                        'same-run held-out' if winner['confirmed'] else 'exploratory')
            if max(r['point']['workers'] for r in group) < max(manifest['workers']):
                evidence += '; higher counts lack usable evidence'
            if p['workers'] == max(manifest['workers']):
                evidence += '; worker limit reached'
            fraction = f"{winner['rate']*cost/1e6/p['workers']:.1%}" if cost else '—'
            lines.append(f"| {mode} | {cost_label(cost)} | {p['workers']} | {p['batch']:,} | "
                         f"{winner['rate']:,.0f} | {fraction} | {winner['batches_per_second']:,.1f} | {evidence} |")
            path = directory/f"selected-{mode}-{cost:g}us.toml"
            path.write_text('\n'.join(f'{k} = {json.dumps(v)}' for k,v in winner['settings'].items() if k in {'fixed_batch_size','max_batch_size','target_batch_eval_ms'})+'\n')
    bench.write_json(directory/'selected.json', selected)
    if plots:
        draw_candidates(directory, manifest, selected)
    lines += ['', '## Scope and artifacts', '',
        f"Scheduling niceness: {manifest.get('niceness','not recorded')}; one sampler core and {len(manifest['database_cpus'])} database/server cores.", '',
        f"The registered pool contains {max(manifest['workers'])} evaluator processes; plotted counts are active evaluators. "
        f"They share {len(set(manifest['evaluator_cpus']))} physical evaluator cores, with idle processes still registered.", '',
        'Batch caps bound nominal batch work and estimated sample residency. Minimum batch size is 16. '
        'Warmup requires two completed batches per evaluator in aggregate at the new size; measurements span at least six nominal batch durations. '
        + ('Training measurements also span at least two nominal generation cycles at the configured evaluator count. '
           if manifest.get('minimum_training_generation_windows', 0) >= 2 else '') +
        'A valid interval requires stable assignments, epochs and fresh telemetry; adequacy requires at least eight '
        'accepted batches and eight evaluator completions in aggregate. Busy percentages and RSS are diagnostics, not selection objectives. '
        'Accepted throughput includes ingestion/accumulation and pairs the sampler counter with its own monotonic telemetry interval. '
        'Separately checkpointed run totals remain in the raw CLI data. Short windows can be sensitive to completion boundaries and shared CPU load.', '',
        'Throughput is the objective; within 5% of the peak, prefer fewer evaluators and shorter batches. '
        'Ideal achieved is accepted throughput / (evaluators / configured delay), not CPU efficiency; boundary effects can exceed 100% in short windows. '
        'Search limits or a best point at the maximum evaluator count do not establish saturation. '
        'No multi-host, GPU, optimizer convergence or exclusive-host scaling claim is made. ', '',
        '- `frontier.png` / `.svg`: accepted throughput against evaluator count; dashed lines are ideal service rates, crosses mark unavailable checks (failed or unmeasured, not zero throughput).',
        '- `efficiency.png` / `.svg`: throughput relative to evaluators / delay (zero-delay excluded).',
        '- `batches.png` / `.svg`: selected batch sizes at each measured count.',
        '- `candidates.png` / `.svg`: selected throughput, evaluator count and batch size against evaluation cost.',
        '- `selected-*.toml`: live queue overrides for the corresponding mode/cost run card.',
        '- `manifest.json`, `summary.json`, `results.jsonl`: provenance, decisions and raw results.',
        '- `point-*/`: transition evidence and full performance intervals.',
        '- `<mode>-<cost>us/run.toml` or `confirm-*/run.toml`: exact workload; `harness/` and `gammaboard`: retained executable inputs.', '']
    if manifest.get('sources'):
        lines += ['Source studies (raw intervals and executable inputs remain there):', '']
        lines += [f'- {source}' for source in manifest['sources']]
    if manifest.get('measurement_updates'):
        lines += ['', 'Measurement notes:', '']
        lines += [f'- {note}' for note in manifest['measurement_updates']]
    (directory/'report.md').write_text('\n'.join(lines))
    return directory/'report.md'


def draw(directory, manifest, rows, records):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    plt.rcParams.update({'font.size':10, 'axes.spines.top':False, 'axes.spines.right':False})
    modes = manifest['suite']['modes']; costs = sorted(manifest['suite']['eval_us'])
    for metric, name, ylabel in [('rate','frontier','Accepted samples/s'), ('batch','batches','Samples/batch'), ('efficiency','efficiency','Fraction of ideal delay-limited rate')]:
        fig, axes = plt.subplots(1,len(modes),figsize=(6*len(modes),4.5),squeeze=False,sharey=True)
        has_unavailable = False
        for ax, mode in zip(axes[0],modes):
            for i,cost in enumerate(costs):
                data = sorted((r for r in rows if r['point']['mode']==mode and r['point']['eval_us']==cost),key=lambda r:r['point']['workers'])
                if metric=='efficiency' and cost==0: continue
                x = [r['point']['workers'] for r in data]
                y = [r['point']['batch'] if metric=='batch' else
                     r['rate']*cost/1e6/r['point']['workers'] if metric=='efficiency' else r['rate'] for r in data]
                color = 'black' if cost==0 else f'C{i-int(0 in costs)}'
                ax.plot(x,y,color=color,alpha=.85,lw=2.2 if cost==0 else 1.5,label='0 delay' if cost==0 else cost_label(cost))
                if metric=='rate' and cost:
                    ax.plot(manifest['workers'],[n*1e6/cost for n in manifest['workers']],
                            color=color,alpha=.35,ls='--',lw=1)
                for r,xx,yy in zip(data,x,y):
                    marker = 'o' if r['fresh_run'] or not r['confirmed'] else 's'
                    ax.plot(xx,yy,marker,color=color,markerfacecolor=color if r['fresh_run'] else 'white',ms=6)
                if metric=='rate':
                    failed={r['point']['workers'] for r in records if
                        r['point']['mode']==mode and r['point']['eval_us']==cost and
                        (not r['valid'] or not adequate(r))}
                    planned={p['workers'] for p in manifest.get('validation_points') or [] if
                             p['mode']==mode and p['eval_us']==cost}
                    unavailable=sorted((failed|planned)-set(x))
                    if unavailable:
                        has_unavailable = True
                        ax.plot(unavailable,[.02+.025*i]*len(unavailable),'x',color=color,
                                transform=ax.get_xaxis_transform(),ms=6)
            ax.set(title=mode,xlabel='Evaluators',ylabel=ylabel,yscale='log')
            ax.set_xscale('log',base=2);ax.set_xticks(manifest['workers'],labels=manifest['workers'])
            ax.set_xlim(min(manifest['workers'])*.85,max(manifest['workers'])*1.15)
            if metric=='rate' and rows:
                ax.set_ylim(min(r['rate'] for r in rows if r['rate']>0)*.6,max(r['rate'] for r in rows)*2)
            if metric=='efficiency':
                ax.set_yscale('linear');ax.set_ylim(bottom=0);ax.axhline(1,color='gray',ls=':',lw=1)
            ax.grid(alpha=.2);ax.legend(fontsize=8)
        sigma=manifest.get('timing_jitter',{}).get('relative_sigma',0)
        evidence='fresh-run measurements' if rows and all(r['fresh_run'] for r in rows) else 'filled = fresh validation · open = search'
        caption=f'Shared host · {sigma:.0%} batch jitter · {evidence}'
        if metric=='rate':
            caption+=' · dashed = ideal N / delay'
            if has_unavailable:caption+=' · × at foot = unavailable'
        fig.suptitle(caption,fontsize=10)
        fig.tight_layout()
        for extension in ['png','svg']:fig.savefig(directory/f'{name}.{extension}',dpi=160,bbox_inches='tight')
        plt.close(fig)


def draw_candidates(directory, manifest, selected):
    import matplotlib.pyplot as plt
    costs = sorted(manifest['suite']['eval_us'])
    fig, axes = plt.subplots(3,1,figsize=(9,9),sharex=True)
    for mode in manifest['suite']['modes']:
        group = sorted((r for r in selected if r['point']['mode']==mode),key=lambda r:r['point']['eval_us'])
        for ax,key in zip(axes,['rate','workers','batch']):
            ax.plot([r['point']['eval_us'] for r in group],
                    [r['rate'] if key=='rate' else r['point'][key] for r in group],
                    'o-',label=mode)
    for ax,label in zip(axes,['Accepted samples/s','Selected evaluators','Selected samples/batch']):
        ax.set(yscale='log',ylabel=label)
        ax.set_xscale('symlog',linthresh=.5,linscale=.4)
        ax.grid(alpha=.2)
    axes[0].legend()
    axes[1].set_yscale('log',base=2)
    axes[1].set_yticks(manifest['workers'],labels=manifest['workers'])
    axes[-1].set_xlim(left=-.05,right=max(.5,max(costs))*1.15)
    axes[-1].set_xticks(costs,labels=[cost_label(c) for c in costs])
    axes[-1].set_xlabel('Simulated evaluation delay per sample (0 = no sleep)')
    fig.suptitle('Selected measured configurations · short intervals on a shared host')
    fig.tight_layout()
    for ext in ['png','svg']:fig.savefig(directory/f'candidates.{ext}',dpi=160,bbox_inches='tight')
    plt.close(fig)
