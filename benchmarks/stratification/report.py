#!/usr/bin/env python3
"""Summarize production Symbolica/Havana repetitions and plot the path and errors."""
import argparse
import json
from pathlib import Path
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

parser=argparse.ArgumentParser()
parser.add_argument('results', type=Path)
parser.add_argument('--output-prefix', default='')
args=parser.parse_args()
data=json.loads(args.results.read_text())
directory=args.results.parent
summary={'configuration':data['configuration'], 'exact_integral':data['integrand']['exact_integral'], 'arms':{}}
arrays={}
for arm in ('ordinary','stratified'):
    records=[r for r in data['records'] if r['arm']==arm]
    estimates=np.array([r['estimate'] for r in records]);errors=np.array([r['stderr'] for r in records])
    arrays[arm]=(estimates,errors)
    totals=np.array([r['training_seconds']+r['inference_seconds'] for r in records])
    summary['arms'][arm]={
        'repetitions':len(records), 'mean_estimate':estimates.mean(0).tolist(),
        'empirical_stddev':estimates.std(0,ddof=1).tolist(),
        'rms_reported_stderr':np.sqrt((errors**2).mean(0)).tolist(),
        'pull_mean':(estimates/errors).mean(0).tolist(),
        'pull_stddev':(estimates/errors).std(0,ddof=1).tolist(),
        'coverage_95':(abs(estimates/errors)<=1.96).mean(0).tolist(),
        'median_total_seconds':float(np.median(totals)),
        'reported_stderr_min':errors.min(0).tolist(),
        'reported_stderr_max':errors.max(0).tolist(),
    }
    if arm=='stratified':
        attempts=np.array([[s['attempts'] for s in r['strata']] for r in records])
        matches=np.array([[s['matches'] for s in r['strata']] for r in records])
        summary['arms'][arm]['match_fraction']=float(matches.sum()/attempts.sum())
        summary['arms'][arm]['mean_stratum_probability']=np.array([r['probabilities'] for r in records]).mean(0).tolist()
ordinary=arrays['ordinary'][0];stratified=arrays['stratified'][0]
summary['empirical_variance_reduction']=(ordinary.var(0,ddof=1)/stratified.var(0,ddof=1)).tolist()
summary['empirical_stderr_reduction']=np.sqrt(summary['empirical_variance_reduction']).tolist()
# Bootstrap whole paired repetitions, preserving correlations across components.
rng=np.random.default_rng(20261021)
indices=rng.integers(0,len(ordinary),size=(4000,len(ordinary)))
ratios=ordinary[indices].var(1,ddof=1)/stratified[indices].var(1,ddof=1)
summary['variance_reduction_bootstrap_95']=np.quantile(ratios,[.025,.975],axis=0).T.tolist()
(directory/(args.output_prefix+'summary.json')).write_text(json.dumps(summary,indent=2)+'\n')

plt.rcParams.update({'font.size':10,'axes.spines.top':False,'axes.spines.right':False})
fig=plt.figure(figsize=(13,8),layout='constrained')
ax=fig.add_subplot(2,2,1,projection='3d')
t=np.linspace(0,1,4001);x=np.tanh(8*np.cos(2*np.pi*t));y=np.tanh(8*np.sin(2*np.pi*t));z=x*y
ax.plot(x,y,z,color='#315f98',lw=1.8)
centers=np.array(data['integrand']['centers']);ax.scatter(*centers.T,color='#d76035',s=55)
for i,c in enumerate(centers):ax.text(*c,f'  c{i}')
for i in range(4):
    for j in range(i):ax.plot(*centers[[i,j]].T,color='gray',alpha=.25,lw=.8)
ax.set(xlabel='x(t)',ylabel='y(t)',zlabel='z(t)',title='Tetrahedron-like path; centres at the corners')
ax=fig.add_subplot(2,2,2)
positions=np.arange(3);width=.34
for i,(arm,color) in enumerate([('ordinary','#315f98'),('stratified','#d76035')]):
    values=summary['arms'][arm]
    ax.bar(positions+(i-.5)*width,values['empirical_stddev'],width,color=color,label=arm)
    ax.scatter(positions+(i-.5)*width,values['rms_reported_stderr'],marker='_',s=160,c='black',zorder=3)
ax.set(xticks=positions,xticklabels=['x','y','z'],ylabel='Standard deviation / standard error',title='Observed spread; black marks: RMS reported errors')
ax.legend()
ax=fig.add_subplot(2,2,3)
for j,name in enumerate(['x','y','z']):
    values=arrays['stratified'][0][:,j]/arrays['stratified'][1][:,j]
    ax.hist(values,bins=np.linspace(-5,5,26),density=True,histtype='step',lw=1.8,label=name)
normal=np.linspace(-5,5,300);ax.plot(normal,np.exp(-normal**2/2)/np.sqrt(2*np.pi),'k--',lw=1,label='N(0,1)')
ax.set(xlabel='(estimate − exact value) / reported error',ylabel='Density',title='Stratified error calibration')
ax.legend()
ax=fig.add_subplot(2,2,4)
for j,name in enumerate(['x','y','z']):ax.plot(arrays['stratified'][1][:,j],'.',label=name,alpha=.7)
ax.set(xlabel='Repetition',ylabel='Reported standard error',title='Training variability and occasional tail samples')
ax.legend()
cfg=data['configuration'];fig.suptitle(f"Symbolica + Havana: {cfg['trials']} A/B pairs, {cfg['training_samples']:,} training + {cfg['inference_samples']:,} inference attempts per arm",fontsize=13)
fig.savefig(directory/(args.output_prefix+'comparison.png'),dpi=180)
fig.savefig(directory/(args.output_prefix+'comparison.pdf'))
print(json.dumps(summary,indent=2))
