import unittest
import benchmark_io as io

class ActivityTests(unittest.TestCase):
    def test_overlap_and_unequal_worker_windows_are_weighted(self):
        def row(name, seconds, compute, busy):
            metrics=dict(epoch='e',runner_epoch='e',node_uuid=name,task_id='1',
                         busy=dict(elapsed_seconds=seconds,compute_seconds=compute,io_seconds=busy))
            return dict(worker_id=name,metrics=metrics,runtime_metrics=metrics)
        measure=dict(snapshots=[
            dict(evaluators=[row('a',0,0,0),row('b',0,0,0)],samplers=[row('s',0,0,0)]),
            dict(evaluators=[row('a',10,10,8),row('b',5,0,4)],samplers=[row('s',10,5,8)])])
        rates=io.activity(measure)
        self.assertAlmostEqual(rates['evaluator_compute'],100*10/15)
        self.assertEqual(rates['evaluator_io'],80)
        self.assertEqual(rates['sampler_compute'],50)
        self.assertEqual(rates['sampler_io'],80)
        measure['snapshots'][-1]['samplers'][0]['runtime_metrics']['busy']['io_seconds']=11
        with self.assertRaises(ValueError): io.activity(measure)

    def test_invalid_trials_are_retained_without_biasing_medians(self):
        row=dict(workers=64,batch_size=16,inserts=1,valid=True,rate=100,
                 evaluator_compute=10,evaluator_io=80,sampler_compute=10,sampler_io=80)
        result=io.summarize([row,dict(row,valid=False,rate=0)])[0]
        self.assertEqual(result['rate']['median'],100)
        self.assertEqual(result['invalid_trials'],1)

    def test_payload_rate_uses_fixed_batch_size_and_actual_encoded_bytes(self):
        metric=dict(count=2,mean=1024**2,std_dev=0.)
        row=dict(worker_id='s',id=1,runtime_metrics=dict(queue=dict(rolling=dict(
            insert_bundle_payload_bytes_per_batch=metric))))
        measure=dict(samples_per_second=655360,snapshots=[dict(samplers=[row])]*2)
        result=io.payload_throughput(measure,65536)
        self.assertEqual(result['batches_per_second'],10)
        self.assertEqual(result['accepted_input_mib_per_second'],10)
        metric['std_dev']=1
        with self.assertRaises(ValueError): io.payload_throughput(measure,65536)
        metric.update(std_dev=0,count=0)
        with self.assertRaises(ValueError): io.payload_throughput(measure,65536)

if __name__=='__main__': unittest.main()
