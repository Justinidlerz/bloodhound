import { useState } from 'react';
import {
  ManifestSelector,
  type ManifestResource,
  type ManifestSelectorProps,
} from './ManifestSelector.js';
import { ManifestViewer, type ManifestViewerProps } from './ManifestViewer.js';

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
  ...viewerProps
}: ManifestPlaygroundProps) {
  const [resource, setResource] = useState(initialResource);
  return (
    <>
      <ManifestSelector
        {...selectorProps}
        components={viewerProps.components}
        onChange={(resource) => {
          setResource(resource);
          onChange?.(resource);
        }}
      />
      {resource ? (
        <ManifestViewer
          {...viewerProps}
          manifest={resource.manifest}
          manifestUrl={resource.url}
          notices={resource.notices}
        />
      ) : null}
    </>
  );
}
