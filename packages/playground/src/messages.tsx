import { createContext, useContext, type ReactNode } from 'react';
import { englishMessages } from './locales/en.js';

export type PlaygroundMessageKey = keyof typeof englishMessages;
export type PlaygroundMessages = Record<PlaygroundMessageKey, string>;
export type PlaygroundMessageParams = Readonly<Record<string, string | number>>;

const MessagesContext = createContext<PlaygroundMessages>(englishMessages);

export function MessagesProvider({
  messages,
  children,
}: {
  messages: Partial<PlaygroundMessages>;
  children: ReactNode;
}) {
  return (
    <MessagesContext.Provider value={{ ...englishMessages, ...messages }}>
      {children}
    </MessagesContext.Provider>
  );
}

export function useMessages() {
  const messages = useContext(MessagesContext);
  return {
    messages,
    text: (key: PlaygroundMessageKey, params?: PlaygroundMessageParams) =>
      messages[key].replace(/\{(\w+)\}/g, (placeholder, name: string) =>
        String(params?.[name] ?? placeholder),
      ),
    label: (value: string) => messages[`value.${value}` as PlaygroundMessageKey] ?? value,
  };
}
