use super::*;
use tokio::io::AsyncReadExt;
#[tokio::test]
async fn permits_separate_lines_but_bounds_a_line_across_reads() {
    let mut normal = BoundedLines::new(&b"abcd\nx\n"[..], 4);
    let mut output = Vec::new();
    normal.read_to_end(&mut output).await.unwrap();
    assert_eq!(output, b"abcd\nx\n");
    let mut oversized = BoundedLines::new(&b"abcdX\n"[..], 4);
    let mut pair = [0; 2];
    oversized.read_exact(&mut pair).await.unwrap();
    oversized.read_exact(&mut pair).await.unwrap();
    assert_eq!(
        oversized.read_exact(&mut pair).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}
