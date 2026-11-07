import { lazy } from "react";
import WorkspaceTabs, { WorkspacePanel } from "./common/WorkspaceTabs";

const WorkersWorkspace = lazy(() => import("./WorkersWorkspace"));
const SettingsWorkspace = lazy(() => import("./SettingsWorkspace"));

export default function SystemWorkspace({ view = "workers", onViewChange, ...props }) {
  return <>
    <WorkspaceTabs id="system" label="System sections" value={view} onChange={onViewChange}
      tabs={[["workers", "Workers"], ["database", "Database"], ["settings", "Settings"]]} />
    <WorkspacePanel id="system" value="workers" active={view === "workers"}><WorkersWorkspace {...props} /></WorkspacePanel>
    <WorkspacePanel id="system" value="database" active={view === "database"}><SettingsWorkspace section="database" /></WorkspacePanel>
    <WorkspacePanel id="system" value="settings" active={view === "settings"}><SettingsWorkspace section="settings" /></WorkspacePanel>
  </>;
}
