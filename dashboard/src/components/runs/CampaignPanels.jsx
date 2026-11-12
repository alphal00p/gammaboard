import { Alert, FormControl, MenuItem, Select, Stack, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import LoadingStatus from "../common/LoadingStatus";
import PanelCollection from "../panels/PanelCollection";
import { useCampaign } from "../../hooks/useCampaign";

export function CampaignPanelResponse({ resource }) {
  const { data, isFetching, error } = resource;
  const states = (data?.updates ?? []).map((update) => update.panel);
  return <>
    <LoadingStatus loading={isFetching} label={data ? "Refreshing campaign…" : "Loading campaign…"} />
    {error && <Alert severity={data ? "warning" : "error"}>{error.message}{data ? " Showing previously loaded data." : ""}</Alert>}
    <PanelCollection panelSpecs={data?.panels ?? []} panelStates={states} />
  </>;
}
export default function CampaignDiagnostics({ runId, taskId }) {
  const preferenceKey = `campaign-diagnostic-group:${runId}:${taskId}`;
  const [group, setGroup] = useState(() => window.sessionStorage.getItem(preferenceKey));
  const resource = useCampaign({ runId, taskId, section: "diagnostics", group });
  const [knownGroups, setKnownGroups] = useState([]);
  useEffect(() => { if (resource.data?.groups) setKnownGroups(resource.data.groups); }, [resource.data?.groups]);
  const groups = resource.data?.groups ?? knownGroups;
  return <Stack spacing={2}>
    {groups.length > 1 && <Stack direction="row" spacing={1} alignItems="center">
      <Typography component="label" id="campaign-group-label">Compatible children</Typography>
      <FormControl size="small" sx={{ minWidth: 240, maxWidth: "100%" }}>
        <Select labelId="campaign-group-label" value={resource.data?.selected_group ?? group ?? groups[0]?.id ?? ""} onChange={(event) => { setGroup(event.target.value); window.sessionStorage.setItem(preferenceKey,event.target.value); }}>
          {groups.map((g) => <MenuItem key={g.id} value={g.id}>{g.label}</MenuItem>)}
        </Select>
      </FormControl>
    </Stack>}
    <CampaignPanelResponse resource={resource} />
  </Stack>;
}
