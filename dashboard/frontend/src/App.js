import { useState, useEffect } from 'react';
import { LineChart, Line, XAxis, YAxis, Tooltip } from 'recharts';

function App() {
  const [results, setResults] = useState([]);

  useEffect(() => {
  fetch('http://localhost:4000/api/results')
    .then(res => res.json())
    .then(data => {
      if (Array.isArray(data)) {
        setResults(data);
      } else {
        console.error("Expected an array from backend", data);
        setResults([]);
      }
    });
}, []);


  return (
    <div>
      <h1>Gammaboard Dashboard</h1>
      <LineChart width={800} height={400} data={results}>
        <Line type="monotone" dataKey="value" stroke="#8884d8" />
        <XAxis dataKey="created_at" />
        <YAxis />
      </LineChart>
    </div>
  );
}

export default App;
