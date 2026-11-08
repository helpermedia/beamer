interface ReadoutProps {
  // The names of the two columns.
  columns: [string, string];
  // The values in and out, null before anything passed.
  rows: { in: [string, string]; out: [string, string] } | null;
}

// The last note or CC as it came in and went out, in a small table.
function Readout({ columns, rows }: ReadoutProps) {
  const cell = "px-2 py-0.5 text-left";
  const value = (text: string | undefined) => (
    <td className={`${cell} font-medium tabular-nums ${text === undefined ? "text-neutral-600" : "text-paper"}`}>
      {text ?? "-"}
    </td>
  );

  return (
    <table className="text-[0.6875rem]">
      <thead>
        <tr className="text-neutral-500">
          <th className={`${cell} font-normal`}>Last</th>
          <th className={`${cell} font-normal`}>{columns[0]}</th>
          <th className={`${cell} font-normal`}>{columns[1]}</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <th className={`${cell} font-normal text-neutral-400`}>In</th>
          {value(rows?.in[0])}
          {value(rows?.in[1])}
        </tr>
        <tr>
          <th className={`${cell} font-normal text-neutral-400`}>Out</th>
          {value(rows?.out[0])}
          {value(rows?.out[1])}
        </tr>
      </tbody>
    </table>
  );
}

export default Readout;
