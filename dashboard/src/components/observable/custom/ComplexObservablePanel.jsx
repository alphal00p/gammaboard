import SampleChart from "../../SampleChart";

const ComplexObservablePanel = ({ samples, isConnected, hasRun }) => (
  <SampleChart samples={samples} isConnected={isConnected} hasRun={hasRun} mode="complex" />
);

export default ComplexObservablePanel;
