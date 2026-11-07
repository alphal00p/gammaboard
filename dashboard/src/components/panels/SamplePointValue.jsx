import { Typography } from "@mui/material";
import DisclosureField from "../common/DisclosureField";
import { asArray } from "../../utils/collections";
import { formatF64Full, formatScientific } from "../../utils/formatters";

const formatNumber = (value, formatter) =>
  typeof value === "number" && Number.isFinite(value) ? formatter(value) : "n/a";

const SamplePointValue = ({ point }) => {
  const discrete = `[${asArray(point.discrete).join(", ")}]`;
  const continuous = asArray(point.continuous);
  const compact = continuous.map((value) => formatNumber(value, (number) => number.toPrecision(3))).join(", ");
  const full = continuous.map((value) => formatNumber(value, formatF64Full)).join(", ");
  const stratum = point.stratum;
  const prefixText = stratum ? `requested=(${asArray(stratum.requested).join(",")}), actual=(${asArray(stratum.actual).join(",")})` : "";
  return <DisclosureField inline label="Sample point details"
    value={`integrand d=${discrete}, c=[${compact}], total weight=${formatNumber(point.total_weight, formatScientific)}`}>
    <Typography component="pre" variant="caption" sx={{ fontFamily: "monospace" }}>
      {`${prefixText}\nintegrand d=${discrete}\nc=[${full}]\ntotal weight=${formatNumber(point.total_weight, formatF64Full)}\nsampler weight=${formatNumber(point.sampling_weight, formatF64Full)}`}
    </Typography>
  </DisclosureField>;
};
export default SamplePointValue;
