import type { JSXElementConstructor, CSSProperties, ReactNode } from 'react';
import { codeWindow, type Location, type RecordRow } from '@bloodhound-rs/manifest';
import { useMessages } from './messages.js';

interface ControlProps {
  title?: string;
  className?: string;
  style?: CSSProperties;
  'aria-label'?: string;
  disabled?: boolean;
}
export interface ButtonProps extends ControlProps {
  variant?: 'action' | 'toggle' | 'link' | 'text';
  'aria-pressed'?: boolean;
  children?: ReactNode;
  onClick?: () => void;
}
export interface InputProps extends ControlProps {
  value: string;
  placeholder?: string;
  onChange: (value: string) => void;
}
export interface SelectProps extends ControlProps {
  value: string;
  options: { label: string; value: string }[];
  onChange: (value: string) => void;
}
export interface TagProps {
  children?: ReactNode;
  status?: string;
}
export interface TableColumn {
  title: string;
  dataIndex: string;
  render: (value: unknown, record: RecordRow) => ReactNode;
}
export interface TableProps {
  data: RecordRow[];
  columns: TableColumn[];
  rowKey: 'id';
  pagination: false;
  selectedRowId?: string;
}
export interface SourceCodeProps {
  text: string;
  path?: string;
  /** Lines are 1-based, columns are 0-based. */
  location?: Location;
}

/** Pass compatible library exports directly; adapt only components with different props. */
export interface ManifestComponents {
  Button: JSXElementConstructor<ButtonProps>;
  Input: JSXElementConstructor<InputProps>;
  Select: JSXElementConstructor<SelectProps>;
  Tag: JSXElementConstructor<TagProps>;
  Table: JSXElementConstructor<TableProps>;
  SourceCode: JSXElementConstructor<SourceCodeProps>;
}

function Button({ variant = 'action', className = '', ...props }: ButtonProps) {
  return (
    <button type="button" className={`bh-button ${className}`} data-variant={variant} {...props} />
  );
}
function Input({ onChange, ...props }: InputProps) {
  return <input {...props} onChange={(event) => onChange(event.target.value)} />;
}
function Select({ options, onChange, ...props }: SelectProps) {
  return (
    <select {...props} onChange={(event) => onChange(event.target.value)}>
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}
function Tag({ children, status }: TagProps) {
  return (
    <span className="bh-tag" data-status={status}>
      {children}
    </span>
  );
}
function Table({ columns, data, selectedRowId }: TableProps) {
  return (
    <table className="bh-table">
      <thead>
        <tr>
          {columns.map((column) => (
            <th scope="col" key={column.dataIndex}>
              {column.title}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {data.map((row) => (
          <tr key={row.id} aria-selected={row.id === selectedRowId}>
            {columns.map((column) => (
              <td key={column.dataIndex}>{column.render(undefined, row)}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
function SourceCode({ text, location }: SourceCodeProps) {
  const { text: t } = useMessages();
  const view = codeWindow(text, location, 25);
  return (
    <div className="bh-code">
      <small>{t('source.lines', { from: view.from, to: view.to, total: view.total })}</small>
      <pre>
        {view.lines.map((line) => (
          <div key={line.number} data-highlight={line.inRange || undefined}>
            <span className="bh-line-number">{line.number}</span>
            {line.cutBefore ? '…' : ''}
            {line.before}
            <mark>{line.hit}</mark>
            {line.after}
            {line.cutAfter ? '…' : ''}
          </div>
        ))}
      </pre>
    </div>
  );
}

export const defaultComponents: ManifestComponents = {
  Button,
  Input,
  Select,
  Tag,
  Table,
  SourceCode,
};
