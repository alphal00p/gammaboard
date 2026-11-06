import { Box, Stack, Typography } from "@mui/material";

export function PanelHeader({ title, children, sx }) {
  return (
    <Box sx={[{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 1, mb: 2 }, ...(Array.isArray(sx) ? sx : [sx])]}>
      <Typography variant="subtitle1" sx={{ minWidth: 0, overflowWrap: "anywhere" }}>{title}</Typography>
      {children && <Stack direction="row" useFlexGap flexWrap="wrap" spacing={1} alignItems="center"
        sx={{ ml: "auto", maxWidth: "100%", justifyContent: "flex-end" }}>{children}</Stack>}
    </Box>
  );
}

export function ControlBar({ children, sx }) {
  return (
    <Stack direction="row" useFlexGap flexWrap="wrap" alignItems="center" spacing={1}
      sx={[{ rowGap: 1.5, minWidth: 0,
        "& .MuiFormControl-root, & .MuiAutocomplete-root": { maxWidth: "100%" },
        "& .MuiInputBase-root": { minHeight: 40 },
        "& > .MuiButton-root": { minHeight: 40 },
        "& > .MuiFormControlLabel-root": { m: 0 },
      }, ...(Array.isArray(sx) ? sx : [sx])]}>
      {children}
    </Stack>
  );
}
