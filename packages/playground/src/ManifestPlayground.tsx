import { useState } from 'react';
import {
  ManifestSelector,
  type ManifestResource,
  type ManifestSelectorProps,
} from './ManifestSelector.js';
import { ManifestViewer, type ManifestViewerProps } from './ManifestViewer.js';
import { englishMessages } from './locales/en.js';

export interface ManifestPlaygroundProps
  extends Omit<ManifestViewerProps, 'manifest' | 'manifestUrl' | 'notices'> {
  initialResource?: ManifestResource;
  onChange?: (resource: ManifestResource) => void;
  selectorProps?: Pick<ManifestSelectorProps, 'loadManifest' | 'parse' | 'maxBytes'>;
}

/** Ready-to-use composition. Hosts with their own resource selection can use ManifestViewer alone. */
export function ManifestPlayground({
  initialResource,
  onChange,
  selectorProps,
  messages = englishMessages,
  ...viewerProps
}: ManifestPlaygroundProps) {
  const [resource, setResource] = useState(initialResource);
  return (
    <>
      <ManifestSelector
        {...selectorProps}
        components={viewerProps.components}
        messages={messages}
        onChange={(resource) => {
          setResource(resource);
          onChange?.(resource);
        }}
      />
      {resource ? (
        <ManifestViewer
          {...viewerProps}
          messages={messages}
          manifest={resource.manifest}
          manifestUrl={resource.url}
          notices={resource.notices}
        />
      ) : null}
    </>
  );
}
