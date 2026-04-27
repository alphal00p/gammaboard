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

const SelectedRunTomlPanel = ({ run }) => {
  const [copyStatus, setCopyStatus] = useState(null);

  if (!run) {
    return null;
  }

  const copyToml = async () => {
    try {
      await copyToClipboard(run.run_toml || "");
      setCopyStatus({ severity: "success", message: "Run TOML copied." });
    } catch (error) {
      setCopyStatus({ severity: "error", message: error?.message || "Failed to copy run TOML." });
    }
  };

  return (
    <Accordion sx={{ mb: 2 }}>
      <AccordionSummary expandIcon={<ExpandMoreIcon />}>
        <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ width: "100%", pr: 1 }}>
          <Box>
            <Typography variant="h6">Selected Run TOML</Typography>
            <Typography variant="body2" color="text.secondary">
              {run.run_name || "Unnamed run"}
            </Typography>
          </Box>
          <Tooltip title="Copy run TOML">
            <IconButton
              size="small"
              onClick={(event) => {
                event.stopPropagation();
                copyToml();
              }}
            >
              <ContentCopyIcon fontSize="small" />
            </IconButton>
          </Tooltip>
        </Stack>
      </AccordionSummary>
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
          {run.run_toml || "# run TOML unavailable"}
        </Box>
      </AccordionDetails>
    </Accordion>
  );
};

export default SelectedRunTomlPanel;
