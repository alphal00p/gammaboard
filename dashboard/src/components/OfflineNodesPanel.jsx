import { Alert, Box, Button, Paper, Stack, Table, TableBody, TableCell, TableContainer, TableHead, TableRow, Tooltip, Typography } from "@mui/material";
import { useOfflineNodes } from "../hooks/useOfflineNodes";
import { formatDateTime } from "../utils/formatters";
import LoadingStatus from "./common/LoadingStatus";
import StatusChip from "./common/StatusChip";

const actions = [["start", "Start"], ["unassign-offline", "Unassign"], ["cancel-restoration", "Cancel restoration"]];
const bulkActions = [["restore", "Restore saved workers"], ["unassign-offline", "Unassign all"], ["cancel-restoration", "Cancel all restorations"]];

export function OfflineNodeActions({ controller }) {
  if (!controller.authenticated) return null;
  return <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
    {bulkActions.map(([action, label]) => <Tooltip key={action} title={controller.eligible(action).length ? "" : "No offline workers are available for this action."}>
      <span><Button variant="outlined" color={action === "cancel-restoration" ? "warning" : "primary"}
        disabled={!controller.eligible(action).length} onClick={() => controller.act(action)}>{label}</Button></span>
    </Tooltip>)}
  </Stack>;
}

function OfflineNodesContent({ controller, standalone = false }) {
  const { nodes, authenticated, error, isLoading, busy, message, disabledReason, act } = controller;
  return <Box>
    <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>Start launches an individual worker again. Restore saved workers starts only those marked during deployment shutdown. Cancel restoration clears that marker; Unassign releases the run reservation. Records and history are kept.</Typography>
    {standalone && <LoadingStatus loading={isLoading || Boolean(busy)} label={busy || "Loading offline workers…"} />}
    {error && <Alert severity="error">{error.message || "Failed to load offline workers."}</Alert>}
    {message && <Alert severity={message.severity} sx={{ mb: 1 }}>{message.text}</Alert>}
    {!isLoading && !nodes.length && <Alert severity="info">No offline workers.</Alert>}
    {nodes.length > 0 && <TableContainer component={Paper} variant="outlined" sx={{ maxHeight: "60vh" }}>
      <Table size="small" stickyHeader aria-label="Offline nodes"><TableHead><TableRow>
        {["Node", "Status", "Reserved run / role", "Last seen", "Launcher", "Actions"].map((name) => <TableCell key={name}>{name}</TableCell>)}
      </TableRow></TableHead><TableBody>{nodes.map((node) => <TableRow key={node.node_name}>
        <TableCell>{node.node_name}</TableCell><TableCell><Stack direction="row" spacing={0.5} useFlexGap flexWrap="wrap">
          <StatusChip status={node.status} />
          {node.restore_requested && <StatusChip status="saved" label="Saved for restart" color="info" />}
        </Stack></TableCell>
        <TableCell>{node.pool_run_id ? `${node.pool_run_name} #${node.pool_run_id} · ${node.pool_role}` : "Unassigned"}</TableCell>
        <TableCell>{formatDateTime(node.last_seen, "Never connected")}</TableCell><TableCell>{node.backend || "Manual launch"}</TableCell>
        <TableCell>{authenticated && <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
          {actions.map(([action, label]) => <Tooltip key={action} title={disabledReason(node, action) || ""}>
            <span><Button size="small" color={action === "cancel-restoration" ? "warning" : "primary"}
              onClick={() => act(action, node)} disabled={Boolean(disabledReason(node, action))}>
              {label}
            </Button></span>
          </Tooltip>)}
        </Stack>}</TableCell>
      </TableRow>)}</TableBody></Table>
    </TableContainer>}
  </Box>;
}

function OfflineNodesSource() {
  const controller = useOfflineNodes();
  return <OfflineNodesContent controller={controller} standalone />;
}

export default function OfflineNodesPanel({ controller }) {
  return controller ? <OfflineNodesContent controller={controller} /> : <OfflineNodesSource />;
}
