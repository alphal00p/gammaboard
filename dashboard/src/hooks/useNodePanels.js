import { fetchNodePanels } from "../services/api";
import { usePanelResource } from "./usePanelSource";

export const useNodePanels = ({ nodeName, pollMs = 3000 } = {}) =>
  usePanelResource({ id: nodeName, pollMs, fetchById: fetchNodePanels });
