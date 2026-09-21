import schema from "../../../../cli/src/config-schema.json" with {
  type: "json",
};

export function GET() {
  return new Response(
    JSON.stringify(schema),
    {
      headers: new Headers({
        "Content-Type": "application/json",
      }),
    },
  );
}
