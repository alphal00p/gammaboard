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
import { useTemplates } from "../../hooks/useTemplates";
import { copyToClipboard } from "../../utils/clipboard";

const TomlActionDialog = ({
  open,
  title,
  label,
  submitLabel,
  initialValue,
  helperText = null,
  warningText = null,
  templateKind = null,
  templatesEnabled = true,
  allowTemplateDelete = false,
  onTemplateSaved = null,
  onTemplateDeleted = null,
  busy = false,
  error = null,
  templateSelectionStorageKey = null,
  onClose,
  onSubmit,
  submitDisabled = false,
  onDuplicate = null,
  duplicateLabel = "Duplicate task",
  exportName = null,
}) => {
  const [value, setValue] = useState(initialValue || "");
  const [selectedTemplate, setSelectedTemplate] = useState("");
  const [templateBusy, setTemplateBusy] = useState(false);
  const [templateActionBusy, setTemplateActionBusy] = useState(false);
  const [templateError, setTemplateError] = useState(null);
  const [exportNotice, setExportNotice] = useState(null);
  const [saveDialogOpen, setSaveDialogOpen] = useState(false);
  const [saveTemplateName, setSaveTemplateName] = useState("");
  const wasOpenRef = useRef(false);
  const restoreGenerationRef = useRef(0);
  const { templates, load: loadTemplate, save: saveTemplate, remove: deleteTemplate } = useTemplates({
    kind: templateKind,
    enabled: templatesEnabled && Boolean(templateKind),
    onError: (err) => setTemplateError(err?.message || "Failed to load templates."),
  });

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
      setSaveDialogOpen(false);
      return;
    }
    if (wasOpenRef.current) return;
    wasOpenRef.current = true;
    setExportNotice(null);

    const restoreGeneration = restoreGenerationRef.current;
    const canApplyRestore = () => restoreGenerationRef.current === restoreGeneration;
    const restore = async () => {
      const restoredSelection = initialValue ? "" : readStoredSelection();
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
    if (busy || templateBusy || templateActionBusy || saveDialogOpen) return;
    onClose();
  };

  const handleSubmit = async (event) => {
    event.preventDefault();
    if (!submitDisabled && !busy && !templateBusy && !templateActionBusy && value.trim()) {
      await onSubmit?.(value);
    }
  };

  const download = () => {
    const url = URL.createObjectURL(new Blob([value], { type: "application/toml" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = `${exportName.replace(/[^a-zA-Z0-9_.-]/g, "_") || "definition"}.toml`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
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
    if (!templateKind) return;
    const suggested = selectedTemplate || "new-template.toml";
    setSaveTemplateName(suggested);
    setSaveDialogOpen(true);
  };

  const handleSaveDialogClose = () => {
    if (templateActionBusy) return;
    setSaveDialogOpen(false);
  };

  const handleSaveTemplateSubmit = async (event) => {
    event.preventDefault();
    if (!templateKind) return;
    const name = saveTemplateName.trim();
    if (!name) return;
    setTemplateError(null);
    setTemplateActionBusy(true);
    try {
      const saved = await saveTemplate(name, value);
      const savedName = String(saved?.name || name).trim();
      if (savedName) {
        setSelectedTemplate(savedName);
        writeStoredSelection(savedName);
      }
      onTemplateSaved?.(saved, name);
      setSaveDialogOpen(false);
    } catch (err) {
      setTemplateError(err?.message || "Failed to save template.");
    } finally {
      setTemplateActionBusy(false);
    }
  };

  const handleDeleteTemplate = async () => {
    if (!allowTemplateDelete || !selectedTemplate) return;
    if (!window.confirm(`Delete template "${selectedTemplate}"?`)) return;
    setTemplateError(null);
    setTemplateActionBusy(true);
    try {
      await deleteTemplate(selectedTemplate);
      onTemplateDeleted?.(selectedTemplate);
      setSelectedTemplate("");
      writeStoredSelection("");
    } catch (err) {
      setTemplateError(err?.message || "Failed to delete template.");
    } finally {
      setTemplateActionBusy(false);
    }
  };

  return (
    <>
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
              {warningText ? <Alert severity="warning">{warningText}</Alert> : null}
              {templateKind ? (
                <Stack direction={{ xs: "column", md: "row" }} spacing={1} alignItems={{ md: "center" }}>
                  {templatesEnabled && <TextField
                    select
                    fullWidth
                    label="Template"
                    value={selectedTemplate}
                    onChange={handleTemplateChange}
                    disabled={busy || templateBusy || templateActionBusy}
                  >
                    <MenuItem value="">Custom</MenuItem>
                    {templates.map((template) => (
                      <MenuItem key={template} value={template}>
                        {template}
                      </MenuItem>
                    ))}
                  </TextField>}
                  {templateKind ? (
                    <Button variant="outlined" onClick={handleSaveTemplate} disabled={busy || templateActionBusy || templateBusy}>
                      Save as Template
                    </Button>
                  ) : null}
                  {allowTemplateDelete ? (
                    <Button
                      variant="outlined"
                      color="error"
                      onClick={handleDeleteTemplate}
                      disabled={busy || templateActionBusy || templateBusy || !selectedTemplate}
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
                disabled={busy || templateBusy || templateActionBusy}
                InputLabelProps={{ shrink: true }}
              />
              {templateError ? <Alert severity="error">{templateError}</Alert> : null}
              {error ? <Alert severity="error">{error}</Alert> : null}
              {exportNotice && <Alert severity={exportNotice.severity}>{exportNotice.message}</Alert>}
            </Stack>
          </DialogContent>
          <DialogActions sx={{ flexWrap: "wrap", gap: 1, px: 3, pb: 2 }}>
            {exportName != null && <>
              <Button disabled={busy || templateBusy || templateActionBusy || !value.trim()} onClick={async () => {
                try { await copyToClipboard(value); setExportNotice({ severity: "success", message: "TOML copied." }); }
                catch (error) { setExportNotice({ severity: "error", message: error.message }); }
              }}>Copy TOML</Button>
              <Button disabled={busy || templateBusy || templateActionBusy || !value.trim()} onClick={download}>Download</Button>
            </>}
            <Button onClick={handleClose} disabled={busy || templateBusy || templateActionBusy}>
              {exportName != null ? "Close" : "Cancel"}
            </Button>
            {onDuplicate && <Button variant={onSubmit && !submitDisabled ? "outlined" : "contained"}
              disabled={busy || templateBusy || templateActionBusy || !value.trim()} onClick={() => onDuplicate(value)}>
              {duplicateLabel}
            </Button>}
            {onSubmit && <Button type="submit" variant={submitDisabled ? "outlined" : "contained"} disabled={submitDisabled || busy || templateBusy || templateActionBusy || !value.trim()}>
              {submitLabel}
            </Button>}
          </DialogActions>
        </form>
      </Dialog>

      <Dialog open={saveDialogOpen} onClose={handleSaveDialogClose} fullWidth maxWidth="xs">
        <form onSubmit={handleSaveTemplateSubmit}>
          <DialogTitle>Save Template</DialogTitle>
          <DialogContent>
            <Stack spacing={2} sx={{ pt: 1 }}>
              <Typography variant="body2" color="text.secondary">
                Save the current TOML as a reusable template.
              </Typography>
              <TextField
                autoFocus
                fullWidth
                label="Template file name"
                value={saveTemplateName}
                onChange={(event) => setSaveTemplateName(event.target.value)}
                disabled={busy || templateActionBusy}
                helperText="Use a concise .toml file name."
              />
            </Stack>
          </DialogContent>
          <DialogActions>
            <Button onClick={handleSaveDialogClose} disabled={busy || templateActionBusy}>
              Cancel
            </Button>
            <Button type="submit" variant="contained" disabled={templateActionBusy || !saveTemplateName.trim()}>
              Save
            </Button>
          </DialogActions>
        </form>
      </Dialog>
    </>
  );
};

export default TomlActionDialog;
