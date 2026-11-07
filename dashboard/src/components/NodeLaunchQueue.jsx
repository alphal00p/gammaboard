import { Paper, Table, TableBody, TableCell,
  TableContainer, TableHead, TableRow, Typography } from "@mui/material";
import DisclosureField from "./common/DisclosureField";
import { formatDateTime } from "../utils/formatters";
import StatusChip from "./common/StatusChip";

const LaunchTable = ({ requests, label }) => (
  <TableContainer component={Paper} variant="outlined">
    <Table size="small" aria-label={label}>
      <TableHead>
        <TableRow>
          {["ID", "State", "Backend", "Nodes", "Requested", "Submitted", "Created", "Error"].map(
            (heading) => <TableCell key={heading}>{heading}</TableCell>,
          )}
        </TableRow>
      </TableHead>
      <TableBody>
        {requests.map((request) => (
          <TableRow key={request.id}>
            <TableCell>{request.id}</TableCell>
            <TableCell><StatusChip status={request.state} /></TableCell>
            <TableCell>{request.backend}</TableCell>
            <TableCell>{(request.args?.groups || []).flatMap((group) => group.node_names || []).join(", ") || "-"}</TableCell>
            <TableCell>{request.requested_count}</TableCell>
            <TableCell>{request.started_count}</TableCell>
            <TableCell>{formatDateTime(request.created_at, "-")}</TableCell>
            <TableCell>{request.error || "-"}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  </TableContainer>
);

export default function NodeLaunchQueue({ requests }) {
  const outstanding = requests.filter((request) => !["fulfilled", "canceled", "failed"].includes(request.state));
  const history = requests.filter((request) => ["fulfilled", "canceled", "failed"].includes(request.state));
  return (
    <>
      {outstanding.length ? (
        <LaunchTable requests={outstanding} label="node startup queue" />
      ) : (
        <Typography color="text.secondary">No outstanding launch requests.</Typography>
      )}
      {history.length > 0 && (
        <DisclosureField section label={`Launch history (${history.length})`} sx={{ mt: 2 }}>
            <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
              Recent fulfilled, canceled, and failed requests. Failures here are past launch attempts. Fulfilled means all requested workers connected;
              current worker health is shown in the node list. Resumed workers keep their names.
            </Typography>
            <LaunchTable requests={history} label="node launch history" />
        </DisclosureField>
      )}
    </>
  );
}
