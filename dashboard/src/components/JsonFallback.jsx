import { Accordion, AccordionDetails, AccordionSummary, Box, Typography } from "@mui/material";
import { ExpandMore as ExpandMoreIcon } from "@mui/icons-material";

const JsonFallback = ({ title = "JSON Fallback", data, defaultExpanded = false }) => (
  <Accordion disableGutters elevation={0} defaultExpanded={defaultExpanded} sx={{ border: 1, borderColor: "divider" }}>
    <AccordionSummary expandIcon={<ExpandMoreIcon />}>
      <Typography variant="subtitle2" color="text.secondary">
        {title}
      </Typography>
    </AccordionSummary>
    <AccordionDetails sx={{ pt: 0 }}>
      <Box
        component="pre"
        sx={{
          m: 0,
          p: 1.5,
          borderRadius: 1,
          backgroundColor: "background.default",
          overflowX: "auto",
          fontSize: "0.8rem",
        }}
      >
        {JSON.stringify(data ?? null, null, 2)}
      </Box>
    </AccordionDetails>
  </Accordion>
);

export default JsonFallback;
