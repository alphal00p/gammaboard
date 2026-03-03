import SampleChart from "../../SampleChart";

const ComplexObservablePanel = ({ samples, isConnected, hasRun, target }) => (
  <SampleChart samples={samples} isConnected={isConnected} hasRun={hasRun} mode="complex" target={target} />
);

export default ComplexObservablePanel;
