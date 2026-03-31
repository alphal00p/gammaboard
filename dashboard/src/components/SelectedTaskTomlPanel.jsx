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

const SelectedTaskTomlPanel = ({ task }) => {
  const [copyStatus, setCopyStatus] = useState(null);

  if (!task) {
    return null;
  }

  const copyToml = async () => {
    try {
      await copyToClipboard(task.task_toml || "");
      setCopyStatus({ severity: "success", message: "Task TOML copied." });
    } catch (error) {
      setCopyStatus({ severity: "error", message: error?.message || "Failed to copy task TOML." });
    }
  };

  return (
    <Accordion sx={{ mb: 2 }}>
      <AccordionSummary expandIcon={<ExpandMoreIcon />}>
        <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ width: "100%", pr: 1 }}>
          <Box>
            <Typography variant="h6">Selected Task TOML</Typography>
            <Typography variant="body2" color="text.secondary">
              {task.name || "Unnamed task"}
            </Typography>
          </Box>
          <Tooltip title="Copy task TOML">
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
          {task.task_toml || "# task TOML unavailable"}
        </Box>
      </AccordionDetails>
    </Accordion>
  );
};

async function copyToClipboard(text) {
  if (navigator?.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "true");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  textarea.style.pointerEvents = "none";
  document.body.appendChild(textarea);
  textarea.focus();
  textarea.select();

  const successful = document.execCommand("copy");
  document.body.removeChild(textarea);

  if (!successful) {
    throw new Error("Clipboard copy is not available in this browser.");
  }
}

export default SelectedTaskTomlPanel;
