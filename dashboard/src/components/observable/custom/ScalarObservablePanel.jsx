import SampleChart from "../../SampleChart";

const ScalarObservablePanel = ({ samples, isConnected, hasRun }) => (
  <SampleChart samples={samples} isConnected={isConnected} hasRun={hasRun} mode="scalar" />
);

export default ScalarObservablePanel;
