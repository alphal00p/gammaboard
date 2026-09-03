import { useState } from "react";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  IconButton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import ContentCopyIcon from "@mui/icons-material/ContentCopy";
import { copyToClipboard } from "../utils/clipboard";

const SelectedTomlPanel = ({ kind, name, toml }) => {
  const [copyStatus, setCopyStatus] = useState(null);
  const normalizedKind = kind.toLowerCase();

  const copyToml = async () => {
    try {
      await copyToClipboard(toml || "");
      setCopyStatus({ severity: "success", message: `${kind} TOML copied.` });
    } catch (error) {
      setCopyStatus({
        severity: "error",
        message: error?.message || `Failed to copy ${normalizedKind} TOML.`,
      });
    }
  };

  return (
    <Accordion sx={{ mb: 2 }}>
      <Stack direction="row" alignItems="center">
        <AccordionSummary expandIcon={<ExpandMoreIcon />} sx={{ flex: 1 }}>
          <Box>
            <Typography variant="h6">Selected {kind} TOML</Typography>
            <Typography variant="body2" color="text.secondary">
              {name || `Unnamed ${normalizedKind}`}
            </Typography>
          </Box>
        </AccordionSummary>
        <Tooltip title={`Copy ${normalizedKind} TOML`}>
          <IconButton size="small" onClick={copyToml} sx={{ mr: 1 }}>
            <ContentCopyIcon fontSize="small" />
          </IconButton>
        </Tooltip>
      </Stack>
      <AccordionDetails>
        {copyStatus ? (
          <Alert severity={copyStatus.severity} sx={{ mb: 2 }} onClose={() => setCopyStatus(null)}>
            {copyStatus.message}
          </Alert>
        ) : null}
        <Box
          component="pre"
          sx={{
            m: 0,
            overflowX: "auto",
            whiteSpace: "pre-wrap",
            fontFamily: "monospace",
            fontSize: 13,
          }}
        >
          {toml || `# ${normalizedKind} TOML unavailable`}
        </Box>
      </AccordionDetails>
    </Accordion>
  );
};

export default SelectedTomlPanel;
