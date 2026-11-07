import LoadingStatus from "./LoadingStatus";
import RunSelector from "../RunSelector";
import { asArray } from "../../utils/collections";
import EmptyStateCard from "./EmptyStateCard";
import { Box } from "@mui/material";

const RunScopedWorkspace = ({
  runs,
  selectedRun,
  setSelectedRun,
  showChildRuns = false,
  setShowChildRuns = null,
  hasMoreRuns = false,
  loadMoreRuns = null,
  isLoadingMoreRuns = false,
  isLoadingRuns = false,
  noRunsMessage,
  noSelectionMessage,
  headerActions = null,
  children,
}) => {
  const runList = asArray(runs);
  const header = headerActions && <Box sx={{ display: "flex", mb: 1 }}>
    {headerActions}
  </Box>;
  if (runList.length === 0) {
    return (
      <>
        {header}
        <LoadingStatus loading={isLoadingRuns} label="Loading runs…" />
        {!isLoadingRuns && <EmptyStateCard title="No runs available" message={noRunsMessage} />}
      </>
    );
  }

  return (
    <>
      <RunSelector
        actions={headerActions}
        runs={runList}
        selectedRun={selectedRun}
        onRunChange={setSelectedRun}
        showChildRuns={showChildRuns}
        onShowChildRunsChange={setShowChildRuns}
        hasMoreRuns={hasMoreRuns}
        onLoadMoreRuns={loadMoreRuns}
        isLoadingMoreRuns={isLoadingMoreRuns}
      />
      {!selectedRun ? <EmptyStateCard title="Select a run" message={noSelectionMessage} /> : children}
    </>
  );
};

export default RunScopedWorkspace;
