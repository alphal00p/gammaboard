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
import { useEffect, useRef, useState } from "react";

const TomlActionDialog = ({
  open,
  title,
  label,
  submitLabel,
  initialValue,
  helperText = null,
  templates = [],
  loadTemplate = null,
  onSaveTemplate = null,
  onDeleteTemplate = null,
  busy = false,
  error = null,
  templateSelectionStorageKey = null,
  onClose,
  onSubmit,
}) => {
  const [value, setValue] = useState(initialValue || "");
  const [selectedTemplate, setSelectedTemplate] = useState("");
  const [templateBusy, setTemplateBusy] = useState(false);
  const [templateActionBusy, setTemplateActionBusy] = useState(false);
  const [templateError, setTemplateError] = useState(null);
  const wasOpenRef = useRef(false);
  const restoreGenerationRef = useRef(0);

  const canUseStorage = typeof window !== "undefined" && typeof window.localStorage !== "undefined";
  const readStoredSelection = () => {
    if (!templateSelectionStorageKey || !canUseStorage) return "";
    return window.localStorage.getItem(templateSelectionStorageKey) || "";
  };
  const writeStoredSelection = (nextSelection) => {
    if (!templateSelectionStorageKey || !canUseStorage) return;
    if (nextSelection) {
      window.localStorage.setItem(templateSelectionStorageKey, nextSelection);
    } else {
      window.localStorage.removeItem(templateSelectionStorageKey);
    }
  };

  useEffect(() => {
    if (!open) {
      wasOpenRef.current = false;
      restoreGenerationRef.current += 1;
      return;
    }
    if (wasOpenRef.current) return;
    wasOpenRef.current = true;

    const restoreGeneration = restoreGenerationRef.current;
    const canApplyRestore = () => restoreGenerationRef.current === restoreGeneration;
    const restore = async () => {
      const restoredSelection = readStoredSelection();
      setSelectedTemplate(restoredSelection);
      setTemplateError(null);
      if (!restoredSelection || !loadTemplate) {
        setValue(initialValue || "");
        return;
      }
      setTemplateBusy(true);
      try {
        const templateValue = await loadTemplate(restoredSelection);
        if (canApplyRestore()) {
          setValue(templateValue || "");
        }
      } catch (err) {
        if (canApplyRestore()) {
          setTemplateError(err?.message || "Failed to load template.");
          setValue(initialValue || "");
        }
      } finally {
        if (canApplyRestore()) {
          setTemplateBusy(false);
        }
      }
    };
    restore();
  }, [open]);

  const handleClose = () => {
    if (busy || templateActionBusy) return;
    onClose();
  };

  const handleSubmit = async (event) => {
    event.preventDefault();
    await onSubmit(value);
  };

  const handleTemplateChange = async (event) => {
    const nextTemplate = event.target.value;
    setSelectedTemplate(nextTemplate);
    writeStoredSelection(nextTemplate);
    setTemplateError(null);
    if (!nextTemplate) {
      setValue(initialValue || "");
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

  const handleSaveTemplate = async () => {
    if (!onSaveTemplate) return;
    const suggested = selectedTemplate || "new-template.toml";
    const name = window.prompt("Template file name (.toml)", suggested);
    if (!name) return;
    setTemplateError(null);
    setTemplateActionBusy(true);
    try {
      const saved = await onSaveTemplate(name, value);
      const savedName = String(saved?.name || name).trim();
      if (savedName) {
        setSelectedTemplate(savedName);
        writeStoredSelection(savedName);
      }
    } catch (err) {
      setTemplateError(err?.message || "Failed to save template.");
    } finally {
      setTemplateActionBusy(false);
    }
  };

  const handleDeleteTemplate = async () => {
    if (!onDeleteTemplate || !selectedTemplate) return;
    if (!window.confirm(`Delete template "${selectedTemplate}"?`)) return;
    setTemplateError(null);
    setTemplateActionBusy(true);
    try {
      await onDeleteTemplate(selectedTemplate);
      setSelectedTemplate("");
      writeStoredSelection("");
    } catch (err) {
      setTemplateError(err?.message || "Failed to delete template.");
    } finally {
      setTemplateActionBusy(false);
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
              <Stack direction={{ xs: "column", md: "row" }} spacing={1} alignItems={{ md: "center" }}>
                <TextField
                  select
                  fullWidth
                  label="Template"
                  value={selectedTemplate}
                  onChange={handleTemplateChange}
                  disabled={templateActionBusy}
                >
                  <MenuItem value="">Custom</MenuItem>
                  {templates.map((template) => (
                    <MenuItem key={template} value={template}>
                      {template}
                    </MenuItem>
                  ))}
                </TextField>
                {onSaveTemplate ? (
                  <Button variant="outlined" onClick={handleSaveTemplate} disabled={templateActionBusy || templateBusy}>
                    Save as Template
                  </Button>
                ) : null}
                {onDeleteTemplate ? (
                  <Button
                    variant="outlined"
                    color="error"
                    onClick={handleDeleteTemplate}
                    disabled={templateActionBusy || templateBusy || !selectedTemplate}
                  >
                    Delete Template
                  </Button>
                ) : null}
              </Stack>
            ) : null}
            <TextField
              autoFocus
              fullWidth
              multiline
              minRows={14}
              label={label}
              value={value}
              onChange={(event) => setValue(event.target.value)}
              disabled={templateBusy || templateActionBusy}
              InputLabelProps={{ shrink: true }}
            />
            {templateError ? <Alert severity="error">{templateError}</Alert> : null}
            {error ? <Alert severity="error">{error}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={handleClose} disabled={busy || templateActionBusy}>
            Cancel
          </Button>
          <Button type="submit" variant="contained" disabled={busy || templateBusy || templateActionBusy || !value.trim()}>
            {submitLabel}
          </Button>
        </DialogActions>
      </form>
    </Dialog>
  );
};

export default TomlActionDialog;
