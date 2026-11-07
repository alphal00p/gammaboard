import { Box, Typography } from "@mui/material";
import DisclosureField from "../common/DisclosureField";
import { formatF64Full } from "../../utils/formatters";

const isBranch = (value) => value != null && typeof value === "object";
const leafText = (value) => value == null ? "none" : typeof value === "string" ? value || '""' : String(value);
const branchText = (value) => Array.isArray(value)
  ? `${value.length} item${value.length === 1 ? "" : "s"}`
  : `${Object.keys(value).length} field${Object.keys(value).length === 1 ? "" : "s"}`;

function ConfigNode({ label, value, depth, path }) {
  const branch = isBranch(value);
  const entries = branch ? Object.entries(value) : [];
  const details = branch && entries.length ? <>
    {entries.map(([key, child]) => <ConfigNode key={key} path={[...path, key]}
      label={Array.isArray(value) ? `[${key}]` : key} value={child} depth={depth + 1} />)}
  </> : typeof value === "number" && !Number.isInteger(value)
    ? <Typography component="pre">{formatF64Full(value)}</Typography> : null;
  return <Box data-config-path={JSON.stringify(path)}>
    <DisclosureField label={label} value={branch ? branchText(value) : leafText(value)} indent={depth}
      defaultOpen={branch && depth === 0} sx={{ borderBottom: 1, borderColor: "divider" }}>
      {details}
    </DisclosureField>
  </Box>;
}

// Accept the earlier dotted-key response during rolling upgrades as well as
// the new structured values. Maps avoid treating config keys as object setters.
export function configurationEntries(entries) {
  const roots = new Map();
  for (const entry of entries) {
    const keys = entry.key.split(".");
    let children = roots;
    keys.forEach((key, index) => {
      if (!children.has(key)) children.set(key, { key, label: keys.length === 1 ? entry.label : key, children: new Map() });
      const node = children.get(key);
      if (index === keys.length - 1) node.value = entry.value;
      children = node.children;
    });
  }
  const unpack = (node) => node.children.size
    ? Object.fromEntries([...node.children].map(([key, child]) => [key, unpack(child)])) : node.value;
  return [...roots.values()].map((node) => ({ key: node.key, label: node.label, value: unpack(node) }));
}

export default function ConfigurationTree({ entries, value }) {
  const roots = entries ? configurationEntries(entries)
    : Object.entries(value || {}).map(([key, child]) => ({ key, label: key, value: child }));
  return <Box aria-label="Configuration tree" sx={{ "--field-label-width": { xs: "minmax(100px, 1fr)", sm: "minmax(150px, 1fr)" } }}>
    {roots.map((entry) => <ConfigNode key={entry.key} path={[entry.key]} label={entry.label} value={entry.value} depth={0} />)}
  </Box>;
}
