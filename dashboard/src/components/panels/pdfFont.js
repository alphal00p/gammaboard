import fontUrl from "../../assets/DejaVuSans.ttf?url";

// Embed the font so scientific symbols and Unicode titles survive PDF export.
export const registerPdfFont = async (pdf, signal) => {
  const response = await fetch(fontUrl, { signal });
  if (!response.ok) throw new Error("Failed to load the PDF font.");
  const bytes = new Uint8Array(await response.arrayBuffer());
  const chunks = [];
  for (let offset = 0; offset < bytes.length; offset += 8192) {
    chunks.push(String.fromCharCode(...bytes.subarray(offset, offset + 8192)));
  }
  pdf.addFileToVFS("DejaVuSans.ttf", btoa(chunks.join("")));
  pdf.addFont("DejaVuSans.ttf", "GammaBoard", "normal");
  pdf.setFont("GammaBoard", "normal");
};
