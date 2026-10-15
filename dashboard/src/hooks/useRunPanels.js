import { fetchRunPanels } from "../services/api";
import { usePanelResource } from "./usePanelSource";

export const useRunPanels = ({ runId, pollMs = 2000, refreshKey } = {}) =>
  usePanelResource({ id: runId, pollMs, fetchById: fetchRunPanels, refreshKey });
