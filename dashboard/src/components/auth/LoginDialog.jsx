import { Alert, Button, Dialog, DialogActions, DialogContent, DialogTitle, Stack, TextField } from "@mui/material";
import { useState } from "react";
import { useAuth } from "../../auth/AuthProvider";

const LoginDialog = () => {
  const { busy, dialogOpen, error, login, setDialogOpen } = useAuth();
  const [password, setPassword] = useState("");

  const handleClose = () => {
    if (busy) return;
    setDialogOpen(false);
    setPassword("");
  };

  const handleSubmit = async (event) => {
    event.preventDefault();
    const ok = await login(password);
    if (ok) setPassword("");
  };

  return (
    <Dialog open={dialogOpen} onClose={handleClose} fullWidth maxWidth="xs">
      <form onSubmit={handleSubmit}>
        <DialogTitle>Operator Login</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 1 }}>
            <TextField
              autoFocus
              fullWidth
              label="Admin password"
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            {error ? <Alert severity="error">{error}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={handleClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="contained" disabled={busy || !password.trim()}>
            Log In
          </Button>
        </DialogActions>
      </form>
    </Dialog>
  );
};

export default LoginDialog;
