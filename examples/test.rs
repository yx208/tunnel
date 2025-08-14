use std::fs::File;
use image::codecs::jpeg::JpegEncoder;
use image::{imageops, ImageEncoder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 读取图像
    let img = image::open("C:/Users/Hawthorn/Pictures/Wallpaper/light-blue/322/322-2电脑.png")?;

    // let instant = std::time::Instant::now();
    // let nwidth = img.width() / 2;
    // let nheight = img.height() / 2;
    // let change_image = imageops::resize(
    //     &img,
    //     nwidth,
    //     nheight,
    //     imageops::FilterType::Lanczos3
    // );
    // let result = change_image.save("C:/Users/Hawthorn/Pictures/Wallpaper/light-blue/322/pc-4k2.png");
    // 
    // println!("{:?}", instant.elapsed().as_secs());
    
    // 创建输出文件
    // let file = File::create("C:/Users/Hawthorn/Downloads/output.jpg")?;
    // let encoder = JpegEncoder::new_with_quality(file, 80);
    // 
    // let result = encoder.write_image(rgb_image.as_bytes(), nwidth, nheight, image::ExtendedColorType::Rgb8);
    // match result {
    //     Ok(_) => {}
    //     Err(e) => {
    //         println!("{}", e);
    //     }
    // }

    println!("图片已压缩并保存为 output.jpg");
    Ok(())
}