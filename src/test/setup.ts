import '@testing-library/jest-dom/vitest';

// jsdom draws nothing on a canvas and reports getContext as not
// implemented; the Explore map (Slice 4.6.1) checks for a missing context.
HTMLCanvasElement.prototype.getContext = (() =>
  null) as typeof HTMLCanvasElement.prototype.getContext;
