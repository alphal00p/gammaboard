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

const CloneRunDialog = ({ open, initialName, busy = false, error = null, onClose, onSubmit }) => {
  const [newName, setNewName] = useState(initialName);

  useEffect(() => {
    if (!open) return;
    setNewName(initialName);
  }, [initialName, open]);

  const handleClose = () => {
    if (busy) return;
    onClose();
  };

  const handleSubmit = async (event) => {
    event.preventDefault();
    await onSubmit({
      newName,
    });
  };

  return (
    <Dialog open={open} onClose={handleClose} fullWidth maxWidth="sm">
      <form onSubmit={handleSubmit}>
        <DialogTitle>Clone Run</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 1 }}>
            <Typography variant="body2" color="text.secondary">
              Create a new run from the currently selected task (or initial run state when no task snapshot is
              available).
            </Typography>
            <TextField
              autoFocus
              fullWidth
              label="New Run Name"
              value={newName}
              onChange={(event) => setNewName(event.target.value)}
            />
            {error ? <Alert severity="error">{error}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={handleClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="contained" disabled={busy || !newName.trim()}>
            Clone Run
          </Button>
        </DialogActions>
      </form>
    </Dialog>
  );
};

export default CloneRunDialog;
