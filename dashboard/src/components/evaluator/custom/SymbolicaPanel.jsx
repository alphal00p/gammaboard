import { Card, CardContent, Typography } from "@mui/material";
import LatexFormula from "../../LatexFormula";

const SymbolicaPanel = ({ evaluatorInitMetadata }) => {
  const latex = typeof evaluatorInitMetadata?.expr_latex === "string" ? evaluatorInitMetadata.expr_latex : "";

  return (
    <Card>
      <CardContent>
        <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
          Symbolica Expression
        </Typography>
        {latex ? (
          <LatexFormula latex={latex} fallbackPrefix="Expression LaTeX" />
        ) : (
          <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
            no latex metadata available
          </Typography>
        )}
      </CardContent>
    </Card>
  );
};

export default SymbolicaPanel;
