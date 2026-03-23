import {
  Alert,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";

const TomlActionDialog = ({
  open,
  title,
  label,
  submitLabel,
  initialValue,
  helperText = null,
  templates = [],
  loadTemplate = null,
  busy = false,
  error = null,
  onClose,
  onSubmit,
}) => {
  const [value, setValue] = useState(initialValue);
  const [selectedTemplate, setSelectedTemplate] = useState("");
  const [templateBusy, setTemplateBusy] = useState(false);
  const [templateError, setTemplateError] = useState(null);

  useEffect(() => {
    if (open) {
      setValue(initialValue);
      setSelectedTemplate("");
      setTemplateError(null);
    }
  }, [initialValue, open]);

  const handleClose = () => {
    if (busy) return;
    onClose();
  };

  const handleSubmit = async (event) => {
    event.preventDefault();
    await onSubmit(value);
  };

  const handleTemplateChange = async (event) => {
    const nextTemplate = event.target.value;
    setSelectedTemplate(nextTemplate);
    setTemplateError(null);
    if (!nextTemplate) {
      setValue(initialValue);
      return;
    }
    if (!loadTemplate) return;
    setTemplateBusy(true);
    try {
      const templateValue = await loadTemplate(nextTemplate);
      setValue(templateValue);
    } catch (err) {
      setTemplateError(err?.message || "Failed to load template.");
    } finally {
      setTemplateBusy(false);
    }
  };

  return (
    <Dialog open={open} onClose={handleClose} fullWidth maxWidth="md">
      <form onSubmit={handleSubmit}>
        <DialogTitle>{title}</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 1 }}>
            {helperText ? (
              <Typography variant="body2" color="text.secondary">
                {helperText}
              </Typography>
            ) : null}
            {templates.length > 0 ? (
              <TextField select fullWidth label="Template" value={selectedTemplate} onChange={handleTemplateChange}>
                <MenuItem value="">Custom</MenuItem>
                {templates.map((template) => (
                  <MenuItem key={template} value={template}>
                    {template}
                  </MenuItem>
                ))}
              </TextField>
            ) : null}
            <TextField
              autoFocus
              fullWidth
              multiline
              minRows={14}
              label={label}
              value={value}
              onChange={(event) => setValue(event.target.value)}
              disabled={templateBusy}
              InputLabelProps={{ shrink: true }}
            />
            {templateError ? <Alert severity="error">{templateError}</Alert> : null}
            {error ? <Alert severity="error">{error}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={handleClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="contained" disabled={busy || templateBusy || !value.trim()}>
            {submitLabel}
          </Button>
        </DialogActions>
      </form>
    </Dialog>
  );
};

export default TomlActionDialog;
