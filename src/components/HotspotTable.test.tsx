import { render, screen, cleanup } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { HotspotTable } from './HotspotTable';
afterEach(cleanup);
it('labels GC samples independently of business method calls', () => {
  render(<HotspotTable title="GC" valueColumn="bytes" hotspots={[{name:'Update',thread:'Main Thread',totalBytes:120,callCount:12,avgBytes:10,maxBytes:10}]} />);
  expect(screen.getByRole('columnheader', {name:'分配次数'})).toBeTruthy();
  expect(screen.queryByRole('columnheader', {name:'调用次数'})).toBeNull();
  expect(screen.getByText(/不是父方法调用次数/)).toBeTruthy();
});
it('keeps CPU invocation counts distinct', () => {
  render(<HotspotTable title="CPU" valueColumn="ms" hotspots={[{name:'Update',totalMs:2,callCount:1,avgMs:2,maxMs:2}]} />);
  expect(screen.getByRole('columnheader', {name:'调用次数'})).toBeTruthy();
});
