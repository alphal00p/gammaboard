import { useMemo } from "react";
import { Box, Typography, useTheme } from "@mui/material";
import CodeMirror from "@uiw/react-codemirror";
import { StreamLanguage } from "@codemirror/language";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { EditorView } from "@codemirror/view";

const language = StreamLanguage.define(toml);
const basicSetup = { foldGutter: false, foldKeymap: false, autocompletion: false };

export default function TomlEditor({ label, value, onChange, disabled = false }) {
  const theme = useTheme();
  const extensions = useMemo(() => [
    language,
    EditorView.lineWrapping,
    EditorView.contentAttributes.of({ "aria-label": label, "aria-disabled": String(disabled) }),
  ], [label, disabled]);
  return (
    <Box>
      <Typography variant="caption" color="text.secondary">{label}</Typography>
      <Box sx={{ border: "1px solid", borderColor: "divider", borderRadius: 1, overflow: "hidden",
        "&:focus-within": { borderColor: "primary.main" },
        "& .cm-editor": { fontSize: 13 }, "& .cm-editor.cm-focused": { outline: "none" } }}>
        <CodeMirror
          value={value} onChange={onChange} autoFocus
          minHeight="280px" maxHeight="60vh"
          theme={theme.palette.mode} basicSetup={basicSetup} extensions={extensions}
          readOnly={disabled} editable={!disabled} indentWithTab={false}
        />
      </Box>
    </Box>
  );
}
