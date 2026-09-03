import { fetchRunPanels } from "../services/api";
import { usePanelResource } from "./usePanelSource";

export const useRunPanels = ({ runId, pollMs = 5000 } = {}) =>
  usePanelResource({ id: runId, pollMs, fetchById: fetchRunPanels });
