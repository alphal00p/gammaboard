export const formatRunLabel = (run) => {
  if (!run) return "Unknown run";
  return run.run_name ? `${run.run_name} (#${run.run_id})` : `Run #${run.run_id}`;
};

export const formatRunSecondaryLabel = (run) =>
  `${run.run_status} | completed ${(run.batches_completed || 0).toLocaleString()} | queued ${(run.total_batches || 0).toLocaleString()}`;
