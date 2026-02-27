import { useMemo, useState } from "react";
import { Accordion, AccordionDetails, AccordionSummary, Box, Typography } from "@mui/material";
import { ExpandMore as ExpandMoreIcon } from "@mui/icons-material";

const JsonFallback = ({ title = "JSON Fallback", data, defaultExpanded = false }) => {
  const [expanded, setExpanded] = useState(defaultExpanded);

  const serialized = useMemo(() => {
    if (!expanded) return "";
    try {
      return JSON.stringify(data ?? null, null, 2);
    } catch (err) {
      return `Failed to serialize JSON: ${err instanceof Error ? err.message : "unknown error"}`;
    }
  }, [data, expanded]);

  return (
    <Accordion
      disableGutters
      elevation={0}
      expanded={expanded}
      onChange={(_, isExpanded) => setExpanded(isExpanded)}
      sx={{ border: 1, borderColor: "divider" }}
    >
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
          {serialized}
        </Box>
      </AccordionDetails>
    </Accordion>
  );
};

export default JsonFallback;
