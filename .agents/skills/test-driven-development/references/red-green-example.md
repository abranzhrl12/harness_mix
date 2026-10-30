# Minimal RED/GREEN example

Suppose `parseRange` must reject a reversed range.

**RED:** Add the behavior test and run it before changing the implementation.
The failure should say the reversed range was accepted.

```typescript
test('rejects a reversed range', () => {
  expect(() => parseRange('8-3')).toThrow('range start exceeds end');
});
```

**GREEN:** Add only the validation needed to reject that input. Run the test
again and confirm the expected error. Then run the repository's permitted
checks for the changed scope.
