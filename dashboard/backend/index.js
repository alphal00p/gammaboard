const express = require("express");
const cors = require("cors");
const { Pool } = require("pg");
const path = require("path");
require("dotenv").config({ path: path.resolve(__dirname, "../../.env") });

const app = express();
app.use(cors());
app.use(express.json());

const pool = new Pool({
  user: process.env.DB_USER,
  host: process.env.DB_HOST,
  database: process.env.DB_NAME,
  password: process.env.DB_PASSWORD,
  port: process.env.DB_PORT,
});

app.get("/api/results", async (req, res) => {
  try {
    const { rows } = await pool.query("SELECT * FROM results ORDER BY step ASC LIMIT 1000");
    // Ensure rows is an array
    res.json(Array.isArray(rows) ? rows : []);
  } catch (err) {
    console.error(err);
    res.status(500).json([]);
  }
});

const PORT = process.env.PORT || 4000;
app.listen(PORT, () => console.log(`Backend running on http://localhost:${PORT}`));
