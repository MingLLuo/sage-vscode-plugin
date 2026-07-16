export function protocolItemWithUri<T extends { uri: string }>(item: T, uri: string): T {
  return item.uri === uri ? item : { ...item, uri };
}
