import { Box, Typography } from "@mui/material";
import { asArray } from "../../utils/collections";
import { formatF64Full, formatScientific } from "../../utils/formatters";

const formatNumber = (value, formatter) =>
  typeof value === "number" && Number.isFinite(value) ? formatter(value) : "n/a";

const SamplePointValue = ({ point }) => {
  const discrete = `[${asArray(point.discrete).join(", ")}]`;
  const continuous = asArray(point.continuous);
  const compact = continuous.map((value) => formatNumber(value, (number) => number.toPrecision(3))).join(", ");
  const full = continuous.map((value) => formatNumber(value, formatF64Full)).join(", ");
  const weight = formatNumber(point.sampling_weight, formatScientific);
  const fullWeight = formatNumber(point.sampling_weight, formatF64Full);
  return (
    <Box sx={{ minWidth: 0 }}>
      <Box>{`d=${discrete}, c=[${compact}], w=${weight}`}</Box>
      <Box component="details" sx={{ mt: 0.5 }}>
        <Box component="summary" sx={{ cursor: "pointer", fontSize: "0.8rem", color: "text.secondary" }}>
          Full Precision (f64)
        </Box>
        <Typography
          variant="caption"
          sx={{ mt: 0.5, display: "block", fontFamily: "inherit", whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}
        >
          {`d=${discrete}\nc=[${full}]\nw=${fullWeight}`}
        </Typography>
      </Box>
    </Box>
  );
};

export default SamplePointValue;
