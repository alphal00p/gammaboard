import {
  Alert,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
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
  busy = false,
  error = null,
  onClose,
  onSubmit,
}) => {
  const [value, setValue] = useState(initialValue);

  useEffect(() => {
    if (open) {
      setValue(initialValue);
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
            <TextField
              autoFocus
              fullWidth
              multiline
              minRows={14}
              label={label}
              value={value}
              onChange={(event) => setValue(event.target.value)}
              InputLabelProps={{ shrink: true }}
            />
            {error ? <Alert severity="error">{error}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={handleClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="contained" disabled={busy || !value.trim()}>
            {submitLabel}
          </Button>
        </DialogActions>
      </form>
    </Dialog>
  );
};

export default TomlActionDialog;
