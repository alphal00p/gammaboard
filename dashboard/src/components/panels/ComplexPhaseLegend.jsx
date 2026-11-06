import { Box, Typography } from "@mui/material";
import { formatScientific } from "../../utils/formatters";
import { complexPhaseColorForValue } from "./complexColors";

// CSS angles go clockwise from the top; complex arguments go anticlockwise
// from the positive real axis. Use the raster's colour mapping for both keys.
const phaseGradient = `conic-gradient(${Array.from({ length: 25 }, (_, i) => {
  const degrees = i * 15;
  const angle = Math.PI / 2 - degrees * Math.PI / 180;
  return `rgb(${complexPhaseColorForValue(Math.cos(angle), Math.sin(angle), 1).join(",")}) ${degrees}deg`;
}).join(",")})`;
const magnitudeColour = `rgb(${complexPhaseColorForValue(1, 0, 1).join(",")})`;

export default function ComplexPhaseLegend({ maxMagnitude }) {
  return (
    <Box sx={{ display: "grid", gap: 0.75 }}>
      <Typography variant="caption" color="text.secondary">Phase (radians)</Typography>
      <Box role="img" aria-label="Complex phase colour key: 0 right, π/2 up, ±π left, −π/2 down; saturation increases outwards"
        sx={{ position: "relative", height: 112, fontSize: 10, color: "text.secondary" }}>
        <Box sx={{ position: "absolute", left: 18, top: 18, width: 80, height: 80, borderRadius: "50%",
          background: `radial-gradient(circle closest-side, white, transparent), ${phaseGradient}` }} />
        <Box sx={{ position: "absolute", top: 0, left: 0, right: 0, textAlign: "center" }}>π/2</Box>
        <Box sx={{ position: "absolute", top: 51, right: 0 }}>0</Box>
        <Box sx={{ position: "absolute", top: 51, left: 0 }}>±π</Box>
        <Box sx={{ position: "absolute", bottom: 0, left: 0, right: 0, textAlign: "center" }}>−π/2</Box>
      </Box>
      <Typography variant="caption" color="text.secondary">Magnitude |z|</Typography>
      <Box sx={{ height: 10, borderRadius: 0.5, background: `linear-gradient(to right, white, ${maxMagnitude > 0 ? magnitudeColour : "white"})` }} />
      <Box sx={{ display: "flex", justifyContent: "space-between", fontSize: 10, fontFamily: "monospace", color: "text.secondary" }}>
        <span>0</span><span>{formatScientific(maxMagnitude, 3)}</span>
      </Box>
      <Typography variant="caption" color="text.secondary" sx={{ fontSize: 10, lineHeight: 1.3 }}>
        {maxMagnitude > 0 ? "White = zero. Full colour = largest |z| in the full image." : "No nonzero magnitude. Phase is undefined at zero."}
      </Typography>
    </Box>
  );
}
