import { Alert, Box, Container, Tab, Tabs, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import gammaboardLogo from "./assets/gammalooplogo.svg";
import EvaluatorPanel from "./components/EvaluatorPanel";
import LogsWorkspace from "./components/LogsWorkspace";
import PerformanceWorkspace from "./components/PerformanceWorkspace";
import RunInfo from "./components/RunInfo";
import SamplerAggregatorPanel from "./components/SamplerAggregatorPanel";
import TaskOutputPanel from "./components/TaskOutputPanel";
import TaskQueuePanel from "./components/TaskQueuePanel";
import WorkersWorkspace from "./components/WorkersWorkspace";
import RunScopedWorkspace from "./components/common/RunScopedWorkspace";
import { useRunConfigPanels } from "./hooks/useRunConfigPanels";
import { useRuns } from "./hooks/useRuns";
import { useRunTasks } from "./hooks/useRunTasks";
import { useWorkersData } from "./hooks/useWorkersData";
import { asArray } from "./utils/collections";
import { asTaskList, getCurrentTask } from "./utils/tasks";

const DashboardHeader = () => (
  <Box sx={{ mb: 3 }}>
    <Box
      component="img"
      src={gammaboardLogo}
      alt="Gammaboard"
      sx={{ display: "block", width: "min(100%, 320px)", height: "auto", mb: 1 }}
    />
    <Typography variant="body2" color="text.secondary">
      Real-time Monte Carlo simulation monitoring
    </Typography>
  </Box>
);

const RunModeContent = ({ runs, selectedRun }) => {
  const currentRun = runs.find((entry) => entry.run_id === selectedRun);
  const { tasks } = useRunTasks(selectedRun, 2000);
  const { evaluator, sampler } = useRunConfigPanels({ runId: selectedRun, pollMs: 5000 });
  const runWorkersData = useWorkersData({ runId: selectedRun, pollMs: 3000 });
  const [selectedTaskId, setSelectedTaskId] = useState(null);

  useEffect(() => {
    const taskList = asTaskList(tasks);
    if (taskList.length === 0) {
      setSelectedTaskId(null);
      return;
    }
    if (selectedTaskId != null && taskList.some((task) => task.id === selectedTaskId)) {
      return;
    }
    setSelectedTaskId(getCurrentTask(taskList)?.id ?? taskList[0].id ?? null);
  }, [selectedTaskId, tasks]);

  if (!currentRun) {
    return (
      <Alert severity="warning" sx={{ mb: 3 }}>
        Selected run not found in current run list.
      </Alert>
    );
  }

  const taskList = asTaskList(tasks);
  const selectedTask = taskList.find((task) => task.id === selectedTaskId) ?? getCurrentTask(taskList) ?? null;

  return (
    <>
      <RunInfo runId={selectedRun} />
      <TaskQueuePanel tasks={taskList} selectedTaskId={selectedTask?.id ?? null} onSelectTask={setSelectedTaskId} />
      <EvaluatorPanel run={currentRun} panelResponse={evaluator} />
      <TaskOutputPanel key={selectedTask?.id ?? "no-task"} runId={selectedRun} task={selectedTask} />
      <SamplerAggregatorPanel run={currentRun} panelResponse={sampler} />
    </>
  );
};

const RunsWorkspace = ({ runs, selectedRun, setSelectedRun, isConnected }) => (
  <RunScopedWorkspace
    runs={runs}
    selectedRun={selectedRun}
    setSelectedRun={setSelectedRun}
    isConnected={isConnected}
    noRunsMessage="Create a run to start monitoring task output and engine configuration."
    noSelectionMessage="Pick a run to view task-scoped output and run configuration."
  >
    <RunModeContent runs={runs} selectedRun={selectedRun} />
  </RunScopedWorkspace>
);

function App() {
  const { runs, isConnected } = useRuns();
  const workersData = useWorkersData({ runId: null, pollMs: 3000 });
  const [mode, setMode] = useState("runs");
  const [selectedRun, setSelectedRun] = useState(null);
  const [selectedLogRun, setSelectedLogRun] = useState(null);
  const runList = asArray(runs);

  useEffect(() => {
    if (runList.length === 0) {
      setSelectedRun(null);
      setSelectedLogRun(null);
      return;
    }

    if (!selectedRun || !runList.some((run) => run.run_id === selectedRun)) {
      setSelectedRun(runList[0].run_id);
    }

    if (!selectedLogRun || !runList.some((run) => run.run_id === selectedLogRun)) {
      setSelectedLogRun(runList[0].run_id);
    }
  }, [runList, selectedRun, selectedLogRun]);

  return (
    <Container maxWidth="xl" sx={{ py: 3 }}>
      <DashboardHeader />

      <Tabs value={mode} onChange={(_, next) => setMode(next)} sx={{ mb: 3 }}>
        <Tab value="runs" label="Runs" />
        <Tab value="workers" label="Nodes" />
        <Tab value="performance" label="Performance" />
        <Tab value="logs" label="Logs" />
      </Tabs>

      {mode === "runs" ? (
        <RunsWorkspace
          runs={runList}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          isConnected={isConnected}
        />
      ) : mode === "workers" ? (
        <WorkersWorkspace
          workers={workersData.workers}
          isConnected={workersData.isConnected}
          lastUpdate={workersData.lastUpdate}
          error={workersData.error}
        />
      ) : mode === "performance" ? (
        <PerformanceWorkspace
          runs={runList}
          workers={workersData.workers}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          isConnected={isConnected}
        />
      ) : (
        <LogsWorkspace
          runs={runList}
          workers={workersData.workers}
          selectedRun={selectedLogRun}
          setSelectedRun={setSelectedLogRun}
          isConnected={isConnected}
        />
      )}
    </Container>
  );
}

export default App;
