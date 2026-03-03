import SampleChart from "../../SampleChart";

const ScalarObservablePanel = ({ samples, isConnected, hasRun, target }) => (
  <SampleChart samples={samples} isConnected={isConnected} hasRun={hasRun} mode="scalar" target={target} />
);

export default ScalarObservablePanel;
