export function greeting(name: string): string {
  return `Hello from ${name}!`;
}

if (import.meta.main) {
  console.log(greeting("{{name}}"));
}
