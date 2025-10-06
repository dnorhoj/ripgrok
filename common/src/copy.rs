use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, split},
    sync::watch,
};

pub async fn my_copy<R, W>(reader: &mut R, writer: &mut W) -> anyhow::Result<u64>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    const BUFFER_SIZE: usize = 8192;
    let mut buf = [0u8; BUFFER_SIZE];
    let mut total_bytes = 0;

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break; // EOF
        }

        writer.write_all(&buf[..n]).await?;
        total_bytes += n as u64;
    }

    writer.flush().await?;
    Ok(total_bytes)
}

pub async fn my_copy_bidirectional<A, B>(a: A, b: B) -> anyhow::Result<()>
where
    A: AsyncReadExt + AsyncWriteExt + Unpin,
    B: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let (mut ar, mut aw) = split(a);
    let (mut br, mut bw) = split(b);

    tokio::select! {
        _ = my_copy(&mut ar, &mut bw) => {
        }
        _ = my_copy(&mut br, &mut aw) => {
        }
    };

    Ok(())
}

pub async fn my_copy_counting<R, W>(
    reader: &mut R,
    writer: &mut W,
    byte_count: watch::Sender<u64>,
) -> anyhow::Result<u64>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    const BUFFER_SIZE: usize = 8192;
    let mut buf = [0u8; BUFFER_SIZE];
    let mut total_bytes = 0;

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break; // EOF
        }

        writer.write_all(&buf[..n]).await?;
        total_bytes += n as u64;
        byte_count.send(total_bytes).unwrap()
    }

    writer.flush().await?;
    Ok(total_bytes)
}

pub async fn my_copy_bidirectional_counting<A, B>(
    a: A,
    b: B,
    a_to_b: watch::Sender<u64>,
    b_to_a: watch::Sender<u64>,
) -> anyhow::Result<()>
where
    A: AsyncReadExt + AsyncWriteExt + Unpin,
    B: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let (mut ar, mut aw) = split(a);
    let (mut br, mut bw) = split(b);

    tokio::select! {
        _ = my_copy_counting(&mut ar, &mut bw, a_to_b) => {
        }
        _ = my_copy_counting(&mut br, &mut aw, b_to_a) => {
        }
    };

    Ok(())
}
