import { useId, useState } from "react";
import { Box, Typography } from "@mui/material";

// The same title / indicator / value geometry is used for plain fields and
// disclosures. Actions and editable controls stay outside the toggle button.
export default function DisclosureField({ label, value, children, inline = false, section = false,
  defaultOpen = false, open: controlledOpen, onOpenChange, actions, indent = 0, ariaLabel, sx }) {
  const [localOpen, setLocalOpen] = useState(defaultOpen);
  const [visited, setVisited] = useState(defaultOpen);
  const open = controlledOpen ?? localOpen;
  const expandable = children != null;
  const id = useId();
  const toggle = (event) => {
    event.stopPropagation();
    setVisited(true);
    if (onOpenChange) onOpenChange(!open);
    else setLocalOpen(!open);
  };
  return <Box sx={[{ minWidth: 0 }, ...(Array.isArray(sx) ? sx : [sx])]} data-disclosure-field={typeof label === "string" ? label : undefined}>
    <Box sx={{ display: "flex", alignItems: "center", gap: 1, minWidth: 0 }}>
      <Box component={expandable ? "button" : "div"} type={expandable ? "button" : undefined}
        aria-label={expandable ? ariaLabel || (typeof label === "string" ? label : undefined) : undefined}
        aria-expanded={expandable ? open : undefined} aria-controls={expandable ? id : undefined}
        onClick={expandable ? toggle : undefined}
        sx={{ display: "grid", alignItems: "baseline", gap: 0.75, width: "100%", minWidth: 0,
          gridTemplateColumns: inline ? "14px minmax(0, 1fr)" : section && value == null
            ? "minmax(0, 1fr) 14px" : "var(--field-label-width, minmax(100px, 0.85fr)) 14px minmax(0, 1.4fr)",
          font: "inherit", color: "inherit", textAlign: "start", background: "transparent", border: 0,
          borderRadius: 0.5, px: 0, py: inline ? 0.25 : 0.75,
          cursor: expandable ? "pointer" : "default",
          "&:hover": expandable ? { bgcolor: "action.hover" } : {},
          "&:focus-visible": { outline: "2px solid", outlineColor: "primary.main", outlineOffset: 2 },
        }}>
        {!inline && <Typography component="span" variant={section ? "subtitle1" : "body2"}
          sx={{ pl: indent * 1.5, minWidth: 0, overflowWrap: "anywhere", color: section ? "text.primary" : "text.secondary" }}>{label}</Typography>}
        <Box component="span" aria-hidden="true" sx={{ display: "inline-block", alignSelf: "center", textAlign: "right",
          transform: open && expandable ? "rotate(90deg)" : "none", lineHeight: 1, visibility: expandable ? "visible" : "hidden" }}>{expandable ? "▸" : null}</Box>
        {(!section || value != null) && <Box component="span" className="field-value"
          sx={{ minWidth: 0, fontVariantNumeric: "tabular-nums", overflowWrap: "anywhere" }}>{value}</Box>}
      </Box>
      {actions && <Box sx={{ flexShrink: 0 }}>{actions}</Box>}
    </Box>
    {expandable && <Box id={id} hidden={!open} onClick={(event) => event.stopPropagation()} sx={{ minWidth: 0, pt: 0.5, pb: 1,
      "& pre": { m: 0, whiteSpace: "pre-wrap", overflowWrap: "anywhere", fontSize: "0.8rem" } }}>
      {(open || visited) && children}
    </Box>}
  </Box>;
}
